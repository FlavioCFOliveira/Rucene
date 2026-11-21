//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test010Zombies`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{
    Action, Consequence, Scope, ThreadLeakAction, ThreadLeakLingering, ThreadLeakScope,
    ThreadLeakZombies,
};
use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::{Class, Field, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::Rule;
use randomizedtesting::rules::SystemPropertiesRestoreRule;
use randomizedtesting::SysGlobals;

use super::common::{apply_at_place_suite, cls, place_tests, set_place, ALL_PLACES};
use crate::support::{assert_that_bool, assert_that_str, throw_no_message, unit};
use crate::utils::{assert_failure_with_message, assert_failures_contain_seeds};
use crate::with_nested_test_class::{
    get_logging_messages, run_tests, start_zombie_thread, Place, CLASS as WITH_NESTED,
};

const O: &str = "Test010Zombies";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    apply_at_place_suite(O, |b| {
        b.annotate(ThreadLeakScope { value: Scope::Test })
            .annotate(ThreadLeakLingering { linger: 0 })
            .annotate(ThreadLeakAction {
                value: vec![Action::Interrupt],
            })
            .annotate(ThreadLeakZombies {
                value: Consequence::IgnoreRemainingTests,
            })
    })
});

/// Starts a zombie somewhere; every later suite is ignored.
fn check(p: Place) {
    system::set_property(&SysGlobals::sysprop_killwait(), "10");
    system::set_property(&SysGlobals::sysprop_killattempts(), "2");
    set_place(p, || {
        start_zombie_thread("foobarZombie");
    });
    // Run a class spawning zombie threads.
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_failure_with_message(&r, "1 thread leaked");
    assert_failure_with_message(&r, "foobarZombie");
    assert_failures_contain_seeds(&r);
    assert_that_str(get_logging_messages())
        .contains("There are still zombie threads that couldn't be terminated:");
    // Run another suite. Everything should be ignored because of zombie threads.
    for (_, p2) in ALL_PLACES {
        set_place(p2, || throw_no_message(RUNTIME_EXCEPTION));
        let r = run_tests(std::slice::from_ref(&*NESTED));
        assert_that_bool(r.was_successful())
            .as_(format!("At: {p2:?}"))
            .is_true();
    }
}

/// `Test010Zombies extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![place_tests(
        cls(O).extends(&WITH_NESTED).constructor(unit()).field(
            Field::new_instance::<()>(
                "restoreProperties",
                "com.carrotsearch.randomizedtesting.rules.SystemPropertiesRestoreRule",
                |_| Value::TestRule(Arc::new(SystemPropertiesRestoreRule::new())),
            )
            .annotate(Rule),
        ),
        &ALL_PLACES,
        check,
    )
    .build()]
}
