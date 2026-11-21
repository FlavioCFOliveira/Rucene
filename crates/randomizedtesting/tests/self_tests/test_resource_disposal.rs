//! Port of `com.carrotsearch.randomizedtesting.TestResourceDisposal`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::io::Closeable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use randomizedtesting::RandomizedTest;

use crate::support::{
    assert_true, cls, nested, static_hook, test, throw_no_message, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, run_tests, CLASS as WITH_NESTED,
};

const O: &str = "TestResourceDisposal";

/// `static class DummyCloseable implements Closeable`.
#[derive(Default)]
struct DummyCloseable {
    closed: AtomicBool,
}

impl Closeable for DummyCloseable {
    fn close(&self) -> std::io::Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Err(std::io::Error::other("closed twice"));
        }
        Ok(())
    }
}

static TEST_SCOPE: Mutex<Vec<Arc<DummyCloseable>>> = Mutex::new(Vec::new());
static SUITE_SCOPE: Mutex<Vec<Arc<DummyCloseable>>> = Mutex::new(Vec::new());
static ALL_TEST_SCOPE_CLOSED: AtomicBool = AtomicBool::new(false);
static ALL_SUITE_SCOPE_OPEN: AtomicBool = AtomicBool::new(false);

fn test_scope_add() {
    let c = RandomizedTest::close_after_test(Arc::new(DummyCloseable::default()));
    TEST_SCOPE.lock().unwrap_or_else(|e| e.into_inner()).push(c);
}

fn suite_scope_add() {
    let c = RandomizedTest::close_after_suite(Arc::new(DummyCloseable::default()));
    SUITE_SCOPE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(c);
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("clean", BeforeClass, || {
            TEST_SCOPE.lock().unwrap_or_else(|e| e.into_inner()).clear();
            SUITE_SCOPE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            suite_scope_add();
        }))
        .method(static_hook("afterClass", AfterClass, || {
            let all_closed = TEST_SCOPE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .all(|c| c.closed.load(Ordering::SeqCst));
            ALL_TEST_SCOPE_CLOSED.store(all_closed, Ordering::SeqCst);
            let all_open = SUITE_SCOPE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .all(|c| !c.closed.load(Ordering::SeqCst));
            ALL_SUITE_SCOPE_OPEN.store(all_open, Ordering::SeqCst);
        }))
        .method(test("testScopeResource", || {
            assume_running_nested();
            test_scope_add();
            test_scope_add();
        }))
        .method(test("testScopeResourceWithFailures", || {
            assume_running_nested();
            test_scope_add();
            test_scope_add();
            throw_no_message(randomizedtesting::jdk::exception::RUNTIME_EXCEPTION);
        }))
        .method(test("suiteScopeFromTest", || {
            assume_running_nested();
            suite_scope_add();
            suite_scope_add();
        }))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testScope", || {
            assume_running_nested();
            RandomizedTest::close_after_test(RandomizedTest::close_after_test(Arc::new(
                DummyCloseable::default(),
            )));
        }))
        .build()
});

/// `TestResourceDisposal extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testResourceDisposalTestScope", || {
            check_tests_output(3, 0, 1, 0, &NESTED);
            for c in TEST_SCOPE.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                assert_true(c.closed.load(Ordering::SeqCst));
            }
            assert_true(ALL_TEST_SCOPE_CLOSED.load(Ordering::SeqCst));
        }))
        .method(test("testResourceDisposalSuiteScope", || {
            run_tests(std::slice::from_ref(&*NESTED));
            for c in SUITE_SCOPE.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                assert_true(c.closed.load(Ordering::SeqCst));
            }
            assert_true(ALL_SUITE_SCOPE_OPEN.load(Ordering::SeqCst));
        }))
        .method(test("testFailedDisposalBreaksTestCase", || {
            let r = check_tests_output(1, 0, 1, 0, &NESTED2);
            assert_true(
                r.get_failures()[0]
                    .exception()
                    .is_instance("com.carrotsearch.randomizedtesting.ResourceDisposalError"),
            );
        }))
        .build()]
}
