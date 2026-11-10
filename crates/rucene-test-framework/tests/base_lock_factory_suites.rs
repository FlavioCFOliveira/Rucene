//! The `BaseLockFactoryTestCase` suites (rmp #350): the inherited cases run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the lock
//! factories Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/store/`):
//! `TestSingleInstanceLockFactory`, `TestSimpleFSLockFactory`,
//! `TestSleepingLockWrapper` and `TestNativeFSLockFactory`.
//!
//! Each binding also declares its class's own test methods (core-test port,
//! rmp #398): `TestSingleInstanceLockFactory.testDefaultLockFactory`,
//! `TestSimpleFSLockFactory.testDeleteLockFile` and
//! `TestNativeFSLockFactory.testInvalidateLock`/`testInvalidateChannel`/
//! `testLockFileExists`/`testDeleteLockFile`/`testBadPermissions`.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_lock_factory_suite_draws_match_lucene`
//! compares the seeded draws with `BaseSuitesDrawsOracle`'s.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::store::java_nio_file::{
    FileSystem, JavaIoException, OpenOption, Path, SeekableByteChannel, ACCESS_DENIED_EXCEPTION,
};
use rucene::store::{
    Directory, Lock, NativeFSLock, NativeFSLockFactory, SimpleFSLockFactory,
    SingleInstanceLockFactory, SleepingLockWrapper,
};
use rucene_test_framework::mockfile::filter_file_system_provider::{self, install};
use rucene_test_framework::mockfile::filter_path::FilterPath;
use rucene_test_framework::mockfile::{FilterFileSystemProvider, FilterState};
use rucene_test_framework::store::base_directory_test_case::OwnedTestDirectory;
use rucene_test_framework::store::base_lock_factory_test_case::{
    self, BaseLockFactoryTestCase, LockFactoryTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use randomizedtesting::junit::Assert;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::store::java_nio_file::Files;
use rucene::store::ByteBuffersDirectory;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::base_lock_factory_test_case::lock_factory_test;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSingleInstanceLockFactory`: `newDirectory(random(), new
/// SingleInstanceLockFactory())`.
struct TestSingleInstanceLockFactory;

impl BaseLockFactoryTestCase for TestSingleInstanceLockFactory {
    fn get_directory(&self, _path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(OwnedTestDirectory::new(
            LuceneTestCase::new_directory_lock_factory(
                &*LuceneTestCase::random(),
                Arc::new(SingleInstanceLockFactory::new()),
            ),
        )))
    }
}

/// `TestSimpleFSLockFactory`: `newFSDirectory(path,
/// SimpleFSLockFactory.INSTANCE)`.
struct TestSimpleFSLockFactory;

impl BaseLockFactoryTestCase for TestSimpleFSLockFactory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(OwnedTestDirectory::new(
            LuceneTestCase::new_fs_directory_lock_factory(path, Arc::new(SimpleFSLockFactory)),
        )))
    }
}

/// `TestSleepingLockWrapper`.
struct TestSleepingLockWrapper;

impl BaseLockFactoryTestCase for TestSleepingLockWrapper {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        let lock_wait_timeout = TestUtil::next_long(&*LuceneTestCase::random(), 20, 100);
        let poll_interval = TestUtil::next_long(&*LuceneTestCase::random(), 2, 10);

        let which = LuceneTestCase::random().next_int_bound(3);
        let delegate: Box<dyn Directory> = match which {
            0 => Box::new(OwnedTestDirectory::new(
                LuceneTestCase::new_directory_lock_factory(
                    &*LuceneTestCase::random(),
                    Arc::new(SingleInstanceLockFactory::new()),
                ),
            )),
            _ => Box::new(OwnedTestDirectory::new(LuceneTestCase::new_fs_directory(
                path,
            ))),
        };
        Ok(Box::new(SleepingLockWrapper::with_poll_interval(
            delegate,
            lock_wait_timeout,
            poll_interval,
        )?))
    }
}

/// `TestNativeFSLockFactory`: `newFSDirectory(path,
/// NativeFSLockFactory.INSTANCE)`.
struct TestNativeFSLockFactory;

impl BaseLockFactoryTestCase for TestNativeFSLockFactory {
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>> {
        Ok(Box::new(OwnedTestDirectory::new(
            LuceneTestCase::new_fs_directory_lock_factory(path, Arc::new(NativeFSLockFactory)),
        )))
    }
}

// -----------------------------------------------------------------------------
// The classes' own methods (core-test port, rmp #398)
// -----------------------------------------------------------------------------

/// `TestSingleInstanceLockFactory.testDefaultLockFactory()`: basic locking on
/// single instance lock factory (can't create two `IndexWriter`s).
fn single_instance_test_default_lock_factory() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    // `dir.lockFactory instanceof SingleInstanceLockFactory`
    let lock_factory_name = ByteBuffersDirectory::new()
        .lock_factory()
        .directory_type_name();
    Assert::assert_true(lock_factory_name.ends_with("SingleInstanceLockFactory"));

    let writer = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()))),
    )?;

    // Create a 2nd IndexWriter.  This should fail.
    expect_throws("java.io.IOException", || {
        let mut config = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        )));
        config.set_open_mode(OpenMode::APPEND)?;
        IndexWriter::new(Arc::clone(&dir), config)
    });

    writer.close()
}

/// `TestSimpleFSLockFactory.testDeleteLockFile()`: delete the lockfile and
/// test `ensureValid` fails.
fn simple_fs_test_delete_lock_file(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    let mut dir = case.get_directory(&LuceneTestCase::create_temp_dir())?;
    let result = (|| -> Result<()> {
        let lock = dir.obtain_lock("test.lock")?;
        lock.ensure_valid()?;

        if let Err(e) = dir.delete_file("test.lock") {
            // we can't delete a file for some reason, just clean up and assume the test.
            let _ = lock.close();
            LuceneTestCase::assume_no_exception(
                "test requires the ability to delete a locked file",
                Some(&java_throwable(&e)),
            );
        }

        expect_throws("java.io.IOException", || lock.ensure_valid());
        let _ = lock.close();
        Ok(())
    })();
    // Do this in finally clause in case the assumeNoException is false:
    result.and(dir.close())
}

/// `TestNativeFSLockFactory.testLockFileExists()`: `NativeFSLockFactory`
/// works correctly if the lock file exists.
fn native_fs_test_lock_file_exists(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    let temp_dir = LuceneTestCase::create_temp_dir();
    let lock_file = temp_dir.resolve_str("test.lock");
    Files::create_file(&lock_file)?;

    let mut dir = case.get_directory(&temp_dir)?;
    let l = dir.obtain_lock("test.lock")?;
    l.close()?;
    drop(l);
    dir.close()
}

/// `(NativeFSLockFactory.NativeFSLock) lock`.
fn as_native_fs_lock(lock: &dyn Lock) -> &NativeFSLock {
    lock.as_any()
        .and_then(|any| any.downcast_ref::<NativeFSLock>())
        .expect("java.lang.ClassCastException: not a NativeFSLock")
}

/// `TestNativeFSLockFactory.testInvalidateLock()`: release the lock and test
/// `ensureValid` fails.
fn native_fs_test_invalidate_lock(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    let mut dir = case.get_directory(&LuceneTestCase::create_temp_dir())?;
    let lock = dir.obtain_lock("test.lock")?;
    let native = as_native_fs_lock(&*lock);
    native.ensure_valid()?;
    native.lock().release()?;
    expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
        native.ensure_valid()
    });

    // `IOUtils.closeWhileHandlingException(lock)`
    let _ = lock.close();
    dir.close()
}

/// `TestNativeFSLockFactory.testInvalidateChannel()`: close the channel and
/// test `ensureValid` fails.
fn native_fs_test_invalidate_channel(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    let mut dir = case.get_directory(&LuceneTestCase::create_temp_dir())?;
    let lock = dir.obtain_lock("test.lock")?;
    let native = as_native_fs_lock(&*lock);
    native.ensure_valid()?;
    if let Some(channel) = native.channel() {
        channel.close()?;
    }
    expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
        native.ensure_valid()
    });

    // `IOUtils.closeWhileHandlingException(lock)`
    let _ = lock.close();
    dir.close()
}

/// `TestNativeFSLockFactory.testDeleteLockFile()`: delete the lockfile and
/// test `ensureValid` fails.
fn native_fs_test_delete_lock_file(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    let mut dir = case.get_directory(&LuceneTestCase::create_temp_dir())?;
    let result = (|| -> Result<()> {
        LuceneTestCase::assume_false(
            "we must be able to delete an open file",
            TestUtil::has_windows_fs(&*dir)?,
        );

        let lock = dir.obtain_lock("test.lock")?;
        lock.ensure_valid()?;

        dir.delete_file("test.lock")?;

        expect_throws("java.io.IOException", || lock.ensure_valid());
        let _ = lock.close();
        Ok(())
    })();
    result.and(dir.close())
}

/// `TestNativeFSLockFactory.MockBadPermissionsFileSystem`: a mock filesystem
/// that throws `AccessDeniedException` on creating `test.lock`.
struct MockBadPermissionsFileSystem {
    filter: FilterState,
}

impl MockBadPermissionsFileSystem {
    /// `MockBadPermissionsFileSystem(FileSystem delegateInstance)`.
    fn new(delegate_instance: Arc<dyn FileSystem>) -> Arc<MockBadPermissionsFileSystem> {
        let this = Arc::new(MockBadPermissionsFileSystem {
            filter: FilterState::new("mockbadpermissions://", &delegate_instance),
        });
        install(&this, delegate_instance);
        this
    }
}

impl FilterFileSystemProvider for MockBadPermissionsFileSystem {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.store.TestNativeFSLockFactory$MockBadPermissionsFileSystem"
    }

    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> std::io::Result<Box<dyn SeekableByteChannel>> {
        if path
            .get_file_name()
            .is_some_and(|name| name.to_string() == "test.lock")
        {
            return Err(JavaIoException::file_system(
                ACCESS_DENIED_EXCEPTION,
                Some(path.to_string()),
                None,
                Some("fake access denied".to_string()),
            )
            .into_io_error());
        }
        filter_file_system_provider::new_byte_channel(self, path, options)
    }
}

rucene_test_framework::impl_file_system_provider!(MockBadPermissionsFileSystem);

/// `TestNativeFSLockFactory.testBadPermissions()`.
fn native_fs_test_bad_permissions(case: &dyn BaseLockFactoryTestCase) -> Result<()> {
    // create a mock filesystem that will throw exc on creating test.lock
    let mut tmp_dir = LuceneTestCase::create_temp_dir();
    tmp_dir = FilterPath::unwrap(&tmp_dir).to_real_path(&[])?;
    let mock =
        MockBadPermissionsFileSystem::new(Arc::clone(tmp_dir.get_file_system())).get_file_system();
    let mock_path = mock.get_path(&tmp_dir.to_string(), &[]);

    // we should get an IOException (typically NoSuchFileException but no guarantee) with
    // our fake AccessDenied added as suppressed.
    let mut dir = case.get_directory(&mock_path.resolve_str("indexDir"))?;
    let expected = expect_throws("java.io.IOException", || dir.obtain_lock("test.lock"));
    // `(AccessDeniedException) expected.getSuppressed()[0]`
    let suppressed = &expected.suppressed()[0];
    Assert::assert_true(suppressed.is_instance(ACCESS_DENIED_EXCEPTION));
    Assert::assert_true(
        suppressed
            .message()
            .is_some_and(|m| m.contains("fake access denied")),
    );

    dir.close()
}

/// A binding class of `BaseLockFactoryTestCase`.
fn binding(name: &str, make: fn() -> LockFactoryTestInstance) -> Arc<Class> {
    binding_with(name, make, Vec::new())
}

/// A binding class of `BaseLockFactoryTestCase` with its own methods.
fn binding_with(
    name: &str,
    make: fn() -> LockFactoryTestInstance,
    methods: Vec<randomizedtesting::jdk::reflect::Method>,
) -> Arc<Class> {
    let mut builder = Class::builder(name)
        .extends(&base_lock_factory_test_case::class())
        .constructor(Constructor::no_args(make));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

fn single_instance_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestSingleInstanceLockFactory",
        || LockFactoryTestInstance::new(TestSingleInstanceLockFactory),
        vec![lock_factory_test("testDefaultLockFactory", |_| {
            single_instance_test_default_lock_factory()
        })],
    )
}

fn simple_fs_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestSimpleFSLockFactory",
        || LockFactoryTestInstance::new(TestSimpleFSLockFactory),
        vec![lock_factory_test(
            "testDeleteLockFile",
            simple_fs_test_delete_lock_file,
        )],
    )
}

fn sleeping_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestSleepingLockWrapper", || {
        LockFactoryTestInstance::new(TestSleepingLockWrapper)
    })
}

fn native_fs_class() -> Arc<Class> {
    binding_with(
        "org.apache.lucene.store.TestNativeFSLockFactory",
        || LockFactoryTestInstance::new(TestNativeFSLockFactory),
        vec![
            lock_factory_test("testLockFileExists", native_fs_test_lock_file_exists),
            lock_factory_test("testInvalidateLock", native_fs_test_invalidate_lock),
            lock_factory_test("testInvalidateChannel", native_fs_test_invalidate_channel),
            lock_factory_test("testDeleteLockFile", native_fs_test_delete_lock_file),
            lock_factory_test("testBadPermissions", native_fs_test_bad_permissions),
        ],
    )
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestLockFactorySuiteDraws
// -----------------------------------------------------------------------------

static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestLockFactorySuiteDraws` of `BaseSuitesDrawsOracle`: the cases on
/// `TestSingleInstanceLockFactory`'s directories.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.store.TestLockFactorySuiteDraws")
        .extends(&base_lock_factory_test_case::class())
        .constructor(Constructor::no_args(|| {
            LockFactoryTestInstance::new(TestSingleInstanceLockFactory)
        }))
        .method(draw_hooks::record_tear_down(
            &DRAWS,
            draw_hooks::lucene_tear_down::<LockFactoryTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&DRAWS))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_lock_factory_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_lock_factory_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_single_instance_lock_factory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(single_instance_class);
}

#[test]
fn test_simple_fs_lock_factory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(simple_fs_class);
}

#[test]
fn test_sleeping_lock_wrapper() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(sleeping_class);
}

#[test]
fn test_native_fs_lock_factory() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(native_fs_class);
}
