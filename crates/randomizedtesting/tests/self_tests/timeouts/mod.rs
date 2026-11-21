//! Package `com.carrotsearch.randomizedtesting.timeouts` of the runner's
//! own tests: timeouts, thread leaks, zombies and uncaught exceptions.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;

mod common;
mod test001_timeout_suite;
mod test002_timeout_method;
mod test003_thread_leaks_method;
mod test004_thread_leaks_suite;
mod test005_thread_leaks_system_threads;
mod test006_timeout_and_thread_leak;
mod test007_uncaught_exceptions;
mod test008_seeds_annotation;
mod test009_timeout_or_not_identical_sequence;
mod test010_zombies;
mod test011_runaway_threads;
mod test012_runaway_threads_killed_at_once;
mod test013_thread_leaks_scope_none;
mod test014_timeout;
mod test015_timeout_override;
mod test016_thread_leaks_custom_filters;
mod test017_thread_leaks_custom_filters_exception;
mod test018_timeout_stacks;
mod test019_thread_leak_group;
mod test020_suite_timeout_stops_tests;
mod test021_suite_timeout_hanging;

/// `timeouts.<TestClass>` entries.
pub fn entries() -> Vec<crate::Entry> {
    vec![
        (
            "timeouts.Test001TimeoutSuite",
            test001_timeout_suite::classes as fn() -> Vec<Arc<Class>>,
        ),
        (
            "timeouts.Test002TimeoutMethod",
            test002_timeout_method::classes,
        ),
        (
            "timeouts.Test003ThreadLeaksMethod",
            test003_thread_leaks_method::classes,
        ),
        (
            "timeouts.Test004ThreadLeaksSuite",
            test004_thread_leaks_suite::classes,
        ),
        (
            "timeouts.Test005ThreadLeaksSystemThreads",
            test005_thread_leaks_system_threads::classes,
        ),
        (
            "timeouts.Test006TimeoutAndThreadLeak",
            test006_timeout_and_thread_leak::classes,
        ),
        (
            "timeouts.Test007UncaughtExceptions",
            test007_uncaught_exceptions::classes,
        ),
        (
            "timeouts.Test008SeedsAnnotation",
            test008_seeds_annotation::classes,
        ),
        (
            "timeouts.Test009TimeoutOrNotIdenticalSequence",
            test009_timeout_or_not_identical_sequence::classes,
        ),
        ("timeouts.Test010Zombies", test010_zombies::classes),
        (
            "timeouts.Test011RunawayThreads",
            test011_runaway_threads::classes,
        ),
        (
            "timeouts.Test012RunawayThreadsKilledAtOnce",
            test012_runaway_threads_killed_at_once::classes,
        ),
        (
            "timeouts.Test013ThreadLeaksScopeNone",
            test013_thread_leaks_scope_none::classes,
        ),
        ("timeouts.Test014Timeout", test014_timeout::classes),
        (
            "timeouts.Test015TimeoutOverride",
            test015_timeout_override::classes,
        ),
        (
            "timeouts.Test016ThreadLeaksCustomFilters",
            test016_thread_leaks_custom_filters::classes,
        ),
        (
            "timeouts.Test017ThreadLeaksCustomFiltersException",
            test017_thread_leaks_custom_filters_exception::classes,
        ),
        (
            "timeouts.Test018TimeoutStacks",
            test018_timeout_stacks::classes,
        ),
        (
            "timeouts.Test019ThreadLeakGroup",
            test019_thread_leak_group::classes,
        ),
        (
            "timeouts.Test020SuiteTimeoutStopsTests",
            test020_suite_timeout_stops_tests::classes,
        ),
        (
            "timeouts.Test021SuiteTimeoutHanging",
            test021_suite_timeout_hanging::classes,
        ),
    ]
}
