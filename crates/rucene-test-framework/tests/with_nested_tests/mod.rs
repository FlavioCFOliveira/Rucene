//! Port of `lucene/test-framework/src/test/org/apache/lucene/tests/util/WithNestedTests.java`:
//! "an abstract test class that prepares nested test classes to run. A
//! nested test class will assume it's executed under control of this class
//! and be ignored otherwise."
//!
//! **Divergence.** The instance fields of `WithNestedTests`
//! (`suppressOutputStreams`, the saved and capturing streams) are kept in a
//! static slot: a declared class's instance is the subclass's own value, and
//! the suites of a binary run one at a time.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Field, Method, Value};
use randomizedtesting::jdk::system::{self, PrintSink};
use randomizedtesting::junit::{
    After, Assert, Assume, Before, ClassRule, JUnitResult, Rule, RuleChain, TestRule,
};
use randomizedtesting::rules::{TestRuleAdapter, TestRuleAdapterRule};
use randomizedtesting::{RandomizedRunner, RandomizedTest, SysGlobals};
use rucene_test_framework::util::lucene_test_case::{LuceneTestCase, SuppressSysoutChecks};
use rucene_test_framework::util::test_rule_ignore_test_suites::NESTED_TEST_SUITE;
use rucene_test_framework::util::test_rule_mark_failure::{
    TestRuleMarkFailure, TestRuleMarkFailureRule,
};
use rucene_test_framework::util::{
    FailureMarker, TestRuleIgnoreAfterMaxFailures, TestRuleIgnoreTestSuites,
    TestRuleRestoreSystemProperties,
};

/// The fields of the running `WithNestedTests` instance.
struct Streams {
    suppress_output_streams: bool,
    prev_sys_out: Option<PrintSink>,
    prev_sys_err: Option<PrintSink>,
    sysout: Arc<Mutex<String>>,
    syserr: Arc<Mutex<String>>,
    marker: Arc<TestRuleMarkFailure>,
}

static STREAMS: Mutex<Option<Streams>> = Mutex::new(None);

/// One suite of this binary at a time: the runner's handlers, `System.out`
/// and the failure counters are process-wide.
pub static SUITES: Mutex<()> = Mutex::new(());

/// The `classRules` adapter of `WithNestedTests`.
struct ClassRulesAdapter {
    prev_rule: Mutex<Option<Arc<TestRuleIgnoreAfterMaxFailures>>>,
}

fn is_property_empty(property_name: &str) -> bool {
    system::get_property(property_name).is_none_or(|v| v.trim().is_empty())
}

impl TestRuleAdapter for ClassRulesAdapter {
    fn before(&self) {
        if !is_property_empty(&SysGlobals::sysprop_testfilter())
            || !is_property_empty(&SysGlobals::sysprop_testclass())
            || !is_property_empty(&SysGlobals::sysprop_testmethod())
            || !is_property_empty(&SysGlobals::sysprop_iterations())
        {
            Assume::assume_true(false);
        }
        RandomizedTest::assume_false(RandomizedRunner::has_zombie_threads());
        let new_rule = Arc::new(TestRuleIgnoreAfterMaxFailures::new(i32::MAX));
        *self.prev_rule.lock().unwrap() = Some(LuceneTestCase::replace_max_failure_rule(new_rule));
        RandomizedTest::assume_false(FailureMarker::had_failures());
    }

    fn after_always(&self, _errors: &mut Vec<Throwable>) {
        if let Some(prev) = self.prev_rule.lock().unwrap().take() {
            LuceneTestCase::replace_max_failure_rule(prev);
        }
        FailureMarker::reset_failures();
    }
}

/// The adapter of `rules` printing the nested streams after a failure.
struct PrintOnFailure;

impl TestRuleAdapter for PrintOnFailure {
    fn after_always(&self, _errors: &mut Vec<Throwable>) {
        let (had, suppress) = {
            let s = STREAMS.lock().unwrap();
            let s = s.as_ref().unwrap();
            (s.marker.had_failures(), s.suppress_output_streams)
        };
        if had && suppress {
            system::out_println(&format!("sysout from nested test: {}\n", get_sys_out()));
            system::out_println(&format!("syserr from nested test: {}", get_sys_err()));
        }
    }
}

/// `class WithNestedTests` with `protected WithNestedTests(suppressOutputStreams)`
/// for the subclass `name`: its `classRules`, `rules`, `before()` and
/// `after()`; the caller adds the test methods.
pub fn with_nested_tests(name: &str, suppress_output_streams: bool) -> ClassBuilder {
    let base = Class::builder("org.apache.lucene.tests.util.WithNestedTests")
        .field(
            Field::new_static("classRules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(RuleChain::outer_rule(Arc::new(TestRuleAdapterRule(
                    Arc::new(ClassRulesAdapter {
                        prev_rule: Mutex::new(None),
                    }),
                )))) as Arc<dyn TestRule>)
            })
            .annotate(ClassRule),
        )
        .field(
            Field::new_instance_any("rules", "org.junit.rules.TestRule", move || {
                let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
                *STREAMS.lock().unwrap() = Some(Streams {
                    suppress_output_streams,
                    prev_sys_out: None,
                    prev_sys_err: None,
                    sysout: Arc::new(Mutex::new(String::new())),
                    syserr: Arc::new(Mutex::new(String::new())),
                    marker: Arc::clone(&marker),
                });
                Value::TestRule(Arc::new(
                    RuleChain::outer_rule(Arc::new(TestRuleAdapterRule(Arc::new(
                        TestRuleRestoreSystemProperties::new(&[
                            TestRuleIgnoreTestSuites::PROPERTY_RUN_NESTED,
                        ]),
                    ))))
                    .around(Arc::new(TestRuleAdapterRule(Arc::new(PrintOnFailure))))
                    .around(Arc::new(TestRuleMarkFailureRule(marker))),
                ) as Arc<dyn TestRule>)
            })
            .annotate(Rule),
        )
        .method(Method::new("before").annotate(Before).body_any(|| {
            {
                let mut s = STREAMS.lock().unwrap();
                let s = s.as_mut().unwrap();
                if s.suppress_output_streams {
                    let (out, err) = (Arc::clone(&s.sysout), Arc::clone(&s.syserr));
                    s.prev_sys_out = system::set_out(Some(Arc::new(move |t: &str| {
                        out.lock().unwrap().push_str(t)
                    })));
                    s.prev_sys_err = system::set_err(Some(Arc::new(move |t: &str| {
                        err.lock().unwrap().push_str(t)
                    })));
                }
            }
            FailureMarker::reset_failures();
            system::set_property(TestRuleIgnoreTestSuites::PROPERTY_RUN_NESTED, "true");
        }))
        .method(Method::new("after").annotate(After).body_any(|| {
            let mut s = STREAMS.lock().unwrap();
            let s = s.as_mut().unwrap();
            if s.suppress_output_streams {
                system::set_out(s.prev_sys_out.take());
                system::set_err(s.prev_sys_err.take());
            }
        }))
        .build();
    Class::builder(name)
        .extends(&base)
        .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(|| ()))
}

/// `WithNestedTests.AbstractNestedTest`: a `LuceneTestCase` that implements
/// `NestedTestSuite`, with `@SuppressSysoutChecks`.
pub fn abstract_nested_test() -> Arc<Class> {
    static CLASS: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.util.WithNestedTests$AbstractNestedTest")
            .extends(&LuceneTestCase::class())
            .annotate(SuppressSysoutChecks {
                bug_url: "WithNestedTests has its own stream capture.".to_string(),
            })
            .annotate(randomizedtesting::jdk::reflect::Marker(&NESTED_TEST_SUITE))
            .build()
    });
    Arc::clone(&CLASS)
}

/// `AbstractNestedTest.isRunningNested()`.
pub fn is_running_nested() -> bool {
    TestRuleIgnoreTestSuites::is_running_nested()
}

/// `getSysOut()`.
pub fn get_sys_out() -> String {
    let s = STREAMS.lock().unwrap();
    let s = s.as_ref().unwrap();
    Assert::assert_true(s.suppress_output_streams);
    let out = s.sysout.lock().unwrap().clone();
    out
}

/// `getSysErr()`.
pub fn get_sys_err() -> String {
    let s = STREAMS.lock().unwrap();
    let s = s.as_ref().unwrap();
    Assert::assert_true(s.suppress_output_streams);
    let err = s.syserr.lock().unwrap().clone();
    err
}

/// `assertFailureCount(expected, result)`.
pub fn assert_failure_count(expected: usize, result: &JUnitResult) {
    if result.failure_count() != expected {
        let mut b = String::new();
        for f in result.failures() {
            b.push_str(&format!(
                "\n\n{}\n{}",
                f.message().unwrap_or("null"),
                f.trace()
            ));
        }
        Assert::assert_false_msg(
            &format!(
                "Expected failures: {expected} but was {}, failures below: {b}",
                result.failure_count()
            ),
            true,
        );
    }
}
