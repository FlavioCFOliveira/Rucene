//! The `BaseLockFactoryTestCase` suites (rmp #350): the inherited cases run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the lock
//! factories Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/store/`):
//! `TestSingleInstanceLockFactory`, `TestSimpleFSLockFactory`,
//! `TestSleepingLockWrapper` and `TestNativeFSLockFactory`.
//!
//! The classes' own extra test methods belong to the core-test port
//! (sprint 7) and are not declared here.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_lock_factory_suite_draws_match_lucene`
//! compares the seeded draws with `BaseSuitesDrawsOracle`'s.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::store::java_nio_file::Path;
use rucene::store::{
    Directory, NativeFSLockFactory, SimpleFSLockFactory, SingleInstanceLockFactory,
    SleepingLockWrapper,
};
use rucene_test_framework::store::base_directory_test_case::OwnedTestDirectory;
use rucene_test_framework::store::base_lock_factory_test_case::{
    self, BaseLockFactoryTestCase, LockFactoryTestInstance,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

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

/// A binding class of `BaseLockFactoryTestCase`.
fn binding(name: &str, make: fn() -> LockFactoryTestInstance) -> Arc<Class> {
    Class::builder(name)
        .extends(&base_lock_factory_test_case::class())
        .constructor(Constructor::no_args(make))
        .build()
}

fn single_instance_class() -> Arc<Class> {
    binding(
        "org.apache.lucene.store.TestSingleInstanceLockFactory",
        || LockFactoryTestInstance::new(TestSingleInstanceLockFactory),
    )
}

fn simple_fs_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestSimpleFSLockFactory", || {
        LockFactoryTestInstance::new(TestSimpleFSLockFactory)
    })
}

fn sleeping_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestSleepingLockWrapper", || {
        LockFactoryTestInstance::new(TestSleepingLockWrapper)
    })
}

fn native_fs_class() -> Arc<Class> {
    binding("org.apache.lucene.store.TestNativeFSLockFactory", || {
        LockFactoryTestInstance::new(TestNativeFSLockFactory)
    })
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
