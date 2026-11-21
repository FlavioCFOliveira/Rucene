//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test011RunawayThreads`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! `ExecutorServiceContextPropagation` runs its task on a thread of
//! `Executors.newCachedThreadPool()` (`Test011RunawayThreads.java:49`); the
//! port runs it on one thread named as the pool names its first thread, the
//! context propagation the test checks being the same.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::ThreadLeakLingering;
use randomizedtesting::jdk::exception::{Throwable, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{BeforeClass, Test};
use randomizedtesting::{seed_from_throwable, RandomizedContext, RandomizedTest};

use super::common::{cls, nested};
use crate::support::{
    assert_equals, assert_true, static_hook, test, throw, try_catch, unit, RANDOMIZED_TEST,
};
use crate::utils::{get_runner_seed, get_seed};
use crate::with_nested_test_class::{
    assume_running_nested, is_running_nested, run_tests, CLASS as WITH_NESTED,
};

const O: &str = "Test011RunawayThreads";

/// `ThreadWithException.startAndJoin()`: runs `body` on a new thread, joins
/// it for at most 5 s and rethrows what it threw.
fn start_and_join(body: impl FnOnce() + Send + 'static) {
    let throwable: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
    let t2 = Arc::clone(&throwable);
    let t = Thread::unnamed(move || {
        if let Err(e) = try_catch(body) {
            *t2.lock().unwrap_or_else(|x| x.into_inner()) = Some(e);
        }
    });
    t.start();
    t.join_millis(5000);
    let thrown = throwable.lock().unwrap_or_else(|x| x.into_inner()).take();
    if let Some(e) = thrown {
        e.throw();
    }
}

fn check_context_seed(seed: i64) {
    let ctx = RandomizedContext::current();
    assert_equals(seed, get_seed(&ctx.get_randomness().expect("a randomness")));
}

/// `final boolean withJoin`.
struct Inst {
    with_join: bool,
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(Constructor::no_args(|| Inst { with_join: true }))
        .constructor(
            Constructor::with_params(&[("boolean", None)], |a| Inst {
                with_join: matches!(
                    a.first(),
                    Some(randomizedtesting::jdk::reflect::Value::Bool(true))
                ),
            })
            .modifiers(modifier::PROTECTED),
        )
        .method(
            Method::new("spinoffAndThrow")
                .annotate(Test::new())
                .body(|i: &mut Inst| {
                    assume_running_nested();
                    let t = Thread::unnamed(|| {
                        RandomizedTest::sleep(500);
                        throw(RUNTIME_EXCEPTION, "spinoff exception");
                    });
                    t.start();
                    if i.with_join {
                        let _ = t.join();
                    }
                }),
        )
        .build()
});

static NESTED_NO_JOIN: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NestedNoJoin")
        .extends(&NESTED)
        .constructor(Constructor::no_args(|| Inst { with_join: false }))
        .build()
});

static NESTED_CLASS_SCOPE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NestedClassScope")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("startThread", BeforeClass, || {
            if !is_running_nested() {
                return;
            }
            Thread::unnamed(|| RandomizedTest::sleep(5000)).start();
        }))
        .method(test("spinoffAndThrow", assume_running_nested))
        .build()
});

/// `Executors.newCachedThreadPool()` pool numbering (`pool-N-thread-1`).
static POOLS: AtomicUsize = AtomicUsize::new(1);

/// `@RunWith(RandomizedRunner.class) Test011RunawayThreads extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        // Java runs this class under RandomizedRunner (@RunWith): the default
        // leak scope, which WithNestedTestClass's declaration replaces.
        .annotate(randomizedtesting::annotations::ThreadLeakScope {
            value: randomizedtesting::annotations::Scope::Test,
        })
        .constructor(unit())
        .method(test("subThreadContextPropagation", || {
            let seed = get_runner_seed();
            start_and_join(move || check_context_seed(seed));
        }))
        .method(
            test("ExecutorServiceContextPropagation", || {
                let seed = get_runner_seed();
                let pool = POOLS.fetch_add(1, Ordering::SeqCst);
                start_and_join_named(&format!("pool-{pool}-thread-1"), move || {
                    check_context_seed(seed)
                });
            })
            .annotate(ThreadLeakLingering { linger: 2000 }),
        )
        .method(test("subUncaughtExceptionInSpunOffThread", || {
            let r = run_tests(std::slice::from_ref(&*NESTED));
            assert_equals(1, r.get_failure_count());
            let f = &r.get_failures()[0];
            let test_exception = f.exception();
            assert_true(seed_from_throwable(test_exception).is_some());
            let thread_exception = test_exception.cause().expect("a cause");
            assert_true(seed_from_throwable(thread_exception).is_some());
        }))
        .method(test("subNotJoined", || {
            let r = run_tests(std::slice::from_ref(&*NESTED_NO_JOIN));
            assert_equals(1, r.get_failure_count());
            assert_true(seed_from_throwable(r.get_failures()[0].exception()).is_some());
        }))
        .method(test("subNotJoinOnClassLevel", || {
            let r = run_tests(std::slice::from_ref(&*NESTED_CLASS_SCOPE));
            assert_equals(1, r.get_failure_count());
            assert_true(seed_from_throwable(r.get_failures()[0].exception()).is_some());
        }))
        .build()]
}

/// `executor.submit(task).get()` on a named pool thread; the task's
/// throwable is rethrown (`ExecutionException.getCause()`).
fn start_and_join_named(name: &str, body: impl FnOnce() + Send + 'static) {
    let throwable: Arc<Mutex<Option<Throwable>>> = Arc::new(Mutex::new(None));
    let t2 = Arc::clone(&throwable);
    let t = Thread::new(name, move || {
        if let Err(e) = try_catch(body) {
            *t2.lock().unwrap_or_else(|x| x.into_inner()) = Some(e);
        }
    });
    t.start();
    let _ = t.join();
    let thrown = throwable.lock().unwrap_or_else(|x| x.into_inner()).take();
    if let Some(e) = thrown {
        e.throw();
    }
}
