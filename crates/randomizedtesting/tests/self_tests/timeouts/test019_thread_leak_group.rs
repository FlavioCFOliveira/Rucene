//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test019ThreadLeakGroup`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, Condvar, LazyLock, Mutex};

use randomizedtesting::annotations::{Group, Scope, ThreadLeakGroup, ThreadLeakScope};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{Thread, ThreadGroup};
use randomizedtesting::junit::After;

use super::common::{cls, nested};
use crate::support::{assert_that_bool, assert_that_list, hook, test, unit, RANDOMIZED_TEST};
use crate::utils::{
    assert_failure_with_message, assert_no_live_threads_containing, get_top_thread_group,
};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test019ThreadLeakGroup";

/// `volatile static Thread t`.
static T: Mutex<Option<Thread>> = Mutex::new(None);

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(ThreadLeakGroup {
            value: Group::TestGroup,
        })
        .constructor(unit())
        .method(test("testLeakOutsideOfGroup", || {
            assume_running_nested();
            let latch = Arc::new((Mutex::new(false), Condvar::new()));
            let l = Arc::clone(&latch);
            let new_top_group = ThreadGroup::new(&get_top_thread_group(), "foobar-group");
            let t = Thread::with_group(&new_top_group, "foobar", move || {
                {
                    let (m, c) = &*l;
                    *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
                    c.notify_all();
                }
                let _ = Thread::try_sleep(5000);
            });
            *T.lock().unwrap_or_else(|e| e.into_inner()) = Some(t.clone());
            t.start();
            let (m, c) = &*latch;
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            while !*g {
                g = c.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&NESTED1)
        .annotate(ThreadLeakGroup { value: Group::Main })
        .constructor(unit())
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .extends(&NESTED1)
        .annotate(ThreadLeakGroup { value: Group::All })
        .constructor(unit())
        .build()
});

fn t_alive() -> bool {
    T.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|t| t.is_alive())
}

/// `Test019ThreadLeakGroup extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testTestGroup", || {
            let r = run_tests(std::slice::from_ref(&*NESTED1));
            assert_that_list(r.get_failures()).is_empty();
            assert_that_bool(t_alive()).is_true();
        }))
        .method(test("testMainGroup", || {
            let r = run_tests(std::slice::from_ref(&*NESTED2));
            assert_that_list(r.get_failures()).is_empty();
            assert_that_bool(t_alive()).is_true();
        }))
        .method(test("testAll", || {
            let r = run_tests(std::slice::from_ref(&*NESTED3));
            assert_no_live_threads_containing("foobar");
            assert_failure_with_message(&r, "1 thread leaked from TEST");
        }))
        .method(hook("cleanup", After, || {
            let t = T.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(t) = t {
                t.interrupt();
                let _ = t.join();
            }
        }))
        .build()]
}
