//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/util/`
//! that exercise `LuceneTestCase`'s rule chains (task #359, util part):
//! `TestPleaseFail`, `TestJUnitRuleOrder`, `TestSetupTeardownChaining`,
//! `TestBeforeAfterOverrides`, `TestCodecReported`, `TestReproduceMessage`
//! (with `SorePoint` and `SoreType`), `TestReproduceMessageWithRepeated`,
//! `TestMaxFailuresRule` and `TestSysoutsLimits`, each run as a suite by the
//! ported runner, nested suites included; and (task #359 part 2)
//! `TestExceptionInBeforeClassHooks`, `TestExpectThrows`,
//! `TestFailIfDirectoryNotClosed`, `TestFailIfUnreferencedFiles`,
//! `TestFloatingPointUlpEquality`, `TestGroupFiltering`, `TestJvmInfo`,
//! `TestLineFileDocs`, `TestRamUsageTesterOnWildAnimals`,
//! `TestSeedFromUncaught`, `TestWorstCaseTestBehavior`,
//! `TestRunWithRestrictedPermissions` and `TestTestObjectInputFilterFactory`,
//! whose outcomes (`framework_tests.tsv`, written by `FrameworkTestsOracle`
//! running Lucene's own classes on the JVM) each suite reproduces.

mod oracle_support;
mod with_nested_tests;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::annotations::{
    Action, AnnotationType, Consequence, Repeat, Scope, TestGroup, ThreadLeakAction,
    ThreadLeakLingering, ThreadLeakScope, ThreadLeakZombies,
};
use randomizedtesting::jdk::exception::{self, Throwable};
use randomizedtesting::jdk::reflect::{
    Class, ClassBuilder, Constructor, Field, JavaAnnotation, Method, Value,
};
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{
    After, AfterClass, Assert, Before, BeforeClass, ClassRule, Description, Failure, JUnitCore,
    JUnitResult, Request, Rule, RuleChain, RunListener, Statement, Test, TestRule,
};
use randomizedtesting::rules::TestRuleAdapterRule;
use randomizedtesting::RandomizedTest;
use rucene_test_framework::util::lucene_test_case::LuceneTestCase;
use rucene_test_framework::util::ram_usage_tester::{JavaObject, RamObject};
use rucene_test_framework::util::test_rule_ignore_test_suites::NESTED_TEST_SUITE;
use rucene_test_framework::util::test_rule_limit_sysouts::{Limit, TestRuleLimitSysouts};
use rucene_test_framework::util::test_rule_mark_failure::{
    TestRuleMarkFailure, TestRuleMarkFailureRule,
};
use rucene_test_framework::util::RamUsageTester;
use rucene_test_framework::util::{TestRuleIgnoreAfterMaxFailures, TestRuleIgnoreTestSuites};
use with_nested_tests::{
    abstract_nested_test, assert_failure_count, get_sys_err, is_running_nested,
    prev_sys_out_println, with_nested_tests, SUITES,
};

/// Runs the outer suite and fails with every failure.
fn run_outer(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let class = builder.build();
    let result = JUnitCore::new().run(&Request::a_class(&class));
    let failures: Vec<String> = result
        .failures()
        .iter()
        .map(|f| format!("{}: {}\n{}", f.description(), f.exception(), f.trace()))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(result.run_count() > 0);
}

fn run_classes(class: &Arc<Class>) -> Arc<JUnitResult> {
    JUnitCore::new().run(&Request::a_class(class))
}

fn test(name: &str, body: impl Fn() + Send + Sync + 'static) -> Method {
    Method::new(name).annotate(Test::new()).body_any(body)
}

/// `TestPleaseFail`: passes unless `please.fail` is set; then each test fails
/// as Java's does.
#[test]
fn test_please_fail() {
    fn class() -> Arc<Class> {
        Class::builder("org.apache.lucene.tests.util.TestPleaseFail")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            // `LuceneJUnit3MethodProvider` finds the public `test*` methods.
            .method(Method::new("testFail").body_any(|| {
                if let Some(v) = system::get_property("please.fail") {
                    Assert::fail(&format!(
                        "(intentional failure) expected null, but was:<{v}>"
                    ));
                }
            }))
            .method(Method::new("testError").body_any(|| {
                if system::get_property("please.fail").is_some() {
                    exception::runtime("(intentional error)");
                }
            }))
            .build()
    }
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    let passing = run_classes(&class());
    assert_eq!(
        (passing.run_count(), passing.failure_count()),
        (2, 0),
        "{:?}",
        passing.failures()
    );
    system::set_property("please.fail", "true");
    let failing = run_classes(&class());
    system::clear_property("please.fail");
    let mut failures: Vec<String> = failing
        .failures()
        .iter()
        .map(|f| f.exception().to_string())
        .collect();
    failures.sort();
    assert_eq!(
        failures,
        [
            "java.lang.AssertionError: (intentional failure) expected null, but was:<true>",
            "java.lang.RuntimeException: (intentional error)",
        ]
    );
    // Both failures are test failures: FailureMarker counted them.
    rucene_test_framework::util::FailureMarker::reset_failures();
}

/// `TestJUnitRuleOrder`.
#[test]
fn test_junit_rule_order() {
    static STACK: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    struct StackRule;
    impl TestRule for StackRule {
        fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
            Statement::new(move || {
                STACK.lock().unwrap().push("@Rule before");
                base.evaluate();
                STACK.lock().unwrap().push("@Rule after");
            })
        }
    }
    let nested = Class::builder("org.apache.lucene.tests.util.TestJUnitRuleOrder$Nested")
        .extends(&abstract_nested_test())
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("before")
                .annotate(Before)
                .body_any(|| STACK.lock().unwrap().push("@Before")),
        )
        .method(
            Method::new("after")
                .annotate(After)
                .body_any(|| STACK.lock().unwrap().push("@After")),
        )
        .field(
            Field::new_instance::<()>("testRule", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(StackRule) as Arc<dyn TestRule>)
            })
            .annotate(Rule),
        )
        .method(test("test", || {}))
        .method(
            Method::new("beforeClassCleanup")
                .annotate(BeforeClass)
                .static_body(|| STACK.lock().unwrap().clear()),
        )
        .method(
            Method::new("afterClassCheck")
                .annotate(AfterClass)
                .static_body(|| STACK.lock().unwrap().push("@AfterClass")),
        )
        .build();
    run_outer(
        with_nested_tests("org.apache.lucene.tests.util.TestJUnitRuleOrder", true).method(test(
            "testRuleOrder",
            move || {
                run_classes(&nested);
                assert_eq!(
                    *STACK.lock().unwrap(),
                    [
                        "@Rule before",
                        "@Before",
                        "@After",
                        "@Rule after",
                        "@AfterClass"
                    ]
                );
            },
        )),
    );
}

/// `TestSetupTeardownChaining`: an override of `setUp` or `tearDown` that
/// does not call `super` fails the test.
#[test]
fn test_setup_teardown_chaining() {
    let setup =
        Class::builder("org.apache.lucene.tests.util.TestSetupTeardownChaining$NestedSetupChain")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("setUp")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| system::out_println("Hello.")),
            )
            .method(test("testMe", || {}))
            .build();
    let teardown = Class::builder(
        "org.apache.lucene.tests.util.TestSetupTeardownChaining$NestedTeardownChain",
    )
    .extends(&abstract_nested_test())
    .constructor(Constructor::no_args(|| ()))
    .method(
        Method::new("tearDown")
            .throws(&["java.lang.Exception"])
            .body_any(|| {}),
    )
    .method(test("testMe", || {}))
    .build();
    run_outer(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestSetupTeardownChaining",
            true,
        )
        .method(test("testSetupChaining", move || {
            let result = run_classes(&setup);
            Assert::assert_equals(1, result.failure_count());
            let failure = &result.failures()[0];
            Assert::assert_true(
                failure
                    .message()
                    .unwrap_or("")
                    .contains("One of the overrides of setUp does not propagate the call."),
            );
        }))
        .method(test("testTeardownChaining", move || {
            let result = run_classes(&teardown);
            Assert::assert_equals(1, result.failure_count());
            let failure = &result.failures()[0];
            Assert::assert_true(
                failure
                    .message()
                    .unwrap_or("")
                    .contains("One of the overrides of tearDown does not propagate the call."),
            );
        })),
    );
}

/// `TestBeforeAfterOverrides`: overriding a `@Before` hook is refused by
/// `NoInstanceHooksOverridesRule`.
#[test]
fn test_before_after_overrides() {
    let before1 = Class::builder("org.apache.lucene.tests.util.TestBeforeAfterOverrides$Before1")
        .extends(&abstract_nested_test())
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("before").annotate(Before).body_any(|| {}))
        .method(Method::new("testEmpty").body_any(|| {}))
        .build();
    let before2 = Class::builder("org.apache.lucene.tests.util.TestBeforeAfterOverrides$Before2")
        .extends(&before1)
        .constructor(Constructor::no_args(|| ()))
        .build();
    let before3 = Class::builder("org.apache.lucene.tests.util.TestBeforeAfterOverrides$Before3")
        .extends(&before2)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("before").annotate(Before).body_any(|| {}))
        .build();
    let check = move || {
        let result = run_classes(&before3);
        Assert::assert_equals(1, result.failure_count());
        Assert::assert_true(
            result.failures()[0]
                .trace()
                .contains("There are overridden methods"),
        );
    };
    let check2 = check.clone();
    run_outer(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestBeforeAfterOverrides",
            true,
        )
        .method(test("testBefore", check))
        .method(test("testAfter", check2)),
    );
}

/// `TestCodecReported`: the reproduce diagnostics name the codec the test
/// ran with.
#[test]
fn test_codec_reported() {
    static CODEC_NAME: Mutex<String> = Mutex::new(String::new());
    let nested1 = Class::builder("org.apache.lucene.tests.util.TestCodecReported$Nested1")
        .extends(&abstract_nested_test())
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("testDummy").body_any(|| {
            *CODEC_NAME.lock().unwrap() = rucene::codecs::get_default().unwrap().name().to_string();
            Assert::fail_without_message();
        }))
        .build();
    run_outer(
        with_nested_tests("org.apache.lucene.tests.util.TestCodecReported", true).method(test(
            "testCorrectCodecReported",
            move || {
                let r = run_classes(&nested1);
                Assert::assert_equals(1, r.failure_count());
                let err = get_sys_err();
                let codec = CODEC_NAME.lock().unwrap().clone();
                Assert::assert_true_msg(&err, err.contains(&format!("codec={codec}")));
            },
        )),
    );
}

/// `SorePoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SorePoint {
    BeforeClass,
    Initializer,
    Rule,
    Before,
    Test,
    After,
    AfterClass,
}

impl SorePoint {
    fn name(self) -> &'static str {
        match self {
            SorePoint::BeforeClass => "BEFORE_CLASS",
            SorePoint::Initializer => "INITIALIZER",
            SorePoint::Rule => "RULE",
            SorePoint::Before => "BEFORE",
            SorePoint::Test => "TEST",
            SorePoint::After => "AFTER",
            SorePoint::AfterClass => "AFTER_CLASS",
        }
    }
}

/// `SoreType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoreType {
    Assumption,
    Failure,
    Error,
}

/// `TestReproduceMessage.where` and `type`.
static SORE: Mutex<Option<(SorePoint, SoreType)>> = Mutex::new(None);

/// `TestReproduceMessage.Nested.triggerOn(pt)`.
fn trigger_on(pt: SorePoint) {
    let sore = *SORE.lock().unwrap();
    if let Some((point, kind)) = sore {
        if point == pt {
            match kind {
                SoreType::Assumption => {
                    LuceneTestCase::assume_true(pt.name(), false);
                    exception::runtime("unreachable")
                }
                SoreType::Error => exception::runtime(pt.name()),
                SoreType::Failure => {
                    Assert::fail(pt.name());
                }
            }
        }
    }
}

/// `TestReproduceMessage.Nested`.
fn reproduce_nested() -> Arc<Class> {
    struct SoreRule;
    impl TestRule for SoreRule {
        fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
            Statement::new(move || {
                trigger_on(SorePoint::Rule);
                base.evaluate();
            })
        }
    }
    Class::builder("org.apache.lucene.tests.util.TestReproduceMessage$Nested")
        .extends(&abstract_nested_test())
        .constructor(Constructor::no_args(|| trigger_on(SorePoint::Initializer)))
        .method(
            Method::new("beforeClass")
                .annotate(BeforeClass)
                .static_body(|| {
                    if is_running_nested() {
                        trigger_on(SorePoint::BeforeClass);
                    }
                }),
        )
        .field(
            Field::new_instance::<()>("rule", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(SoreRule) as Arc<dyn TestRule>)
            })
            .annotate(Rule),
        )
        .method(
            Method::new("before")
                .annotate(Before)
                .body_any(|| trigger_on(SorePoint::Before)),
        )
        .method(test("test", || trigger_on(SorePoint::Test)))
        .method(
            Method::new("after")
                .annotate(After)
                .body_any(|| trigger_on(SorePoint::After)),
        )
        .method(
            Method::new("afterClass")
                .annotate(AfterClass)
                .static_body(|| {
                    if is_running_nested() {
                        trigger_on(SorePoint::AfterClass);
                    }
                }),
        )
        .build()
}

/// `TestReproduceMessage.checkTestName(syserr, expectedName)`.
fn check_test_name(syserr: &str, expected_name: &str) {
    Assert::assert_true_msg(syserr, syserr.contains("NOTE: reproduce with:"));
    Assert::assert_true_msg(
        syserr,
        syserr.contains(&format!(" --tests {expected_name}")),
    );
}

/// `TestReproduceMessage`: assumptions print nothing; failures and errors
/// print the reproduce line, naming the test when one ran.
#[test]
fn test_reproduce_message() {
    use SorePoint::*;
    use SoreType::*;
    let nested = reproduce_nested();
    let mut builder = with_nested_tests("org.apache.lucene.tests.util.TestReproduceMessage", true);
    let cases: [(&str, SoreType, SorePoint); 21] = [
        ("testAssumeBeforeClass", Assumption, BeforeClass),
        ("testAssumeInitializer", Assumption, Initializer),
        ("testAssumeRule", Assumption, Rule),
        ("testAssumeBefore", Assumption, Before),
        ("testAssumeTest", Assumption, Test),
        ("testAssumeAfter", Assumption, After),
        ("testAssumeAfterClass", Assumption, AfterClass),
        ("testFailureBeforeClass", Failure, BeforeClass),
        ("testFailureInitializer", Failure, Initializer),
        ("testFailureRule", Failure, Rule),
        ("testFailureBefore", Failure, Before),
        ("testFailureTest", Failure, Test),
        ("testFailureAfter", Failure, After),
        ("testFailureAfterClass", Failure, AfterClass),
        ("testErrorBeforeClass", Error, BeforeClass),
        ("testErrorInitializer", Error, Initializer),
        ("testErrorRule", Error, Rule),
        ("testErrorBefore", Error, Before),
        ("testErrorTest", Error, Test),
        ("testErrorAfter", Error, After),
        ("testErrorAfterClass", Error, AfterClass),
    ];
    for (name, kind, point) in cases {
        let nested = Arc::clone(&nested);
        builder = builder.method(test(name, move || {
            *SORE.lock().unwrap() = Some((point, kind));
            run_classes(&nested);
            *SORE.lock().unwrap() = None;
            let syserr = get_sys_err();
            match (kind, point) {
                (Assumption, _) => Assert::assert_true_msg(&syserr, syserr.is_empty()),
                (_, BeforeClass | Initializer | AfterClass) => {
                    Assert::assert_true_msg(&syserr, syserr.contains("NOTE: reproduce with:"))
                }
                _ => check_test_name(&syserr, "Nested.test"),
            }
        }));
    }
    run_outer(builder);
}

/// `TestReproduceMessageWithRepeated`: the iteration and seed suffixes of a
/// repeated test are stripped from the reproduce line.
#[test]
fn test_reproduce_message_with_repeated() {
    let nested =
        Class::builder("org.apache.lucene.tests.util.TestReproduceMessageWithRepeated$Nested")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testMe")
                    .annotate(Test::new())
                    .annotate(Repeat {
                        iterations: 10,
                        use_constant_seed: false,
                    })
                    .body_any(|| exception::runtime("bad")),
            )
            .build();
    run_outer(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestReproduceMessageWithRepeated",
            true,
        )
        .method(test("testRepeatedMessage", move || {
            run_classes(&nested);
            check_test_name(&get_sys_err(), "Nested.testMe");
        })),
    );
}

/// `TestMaxFailuresRule`: `testMaxFailures` (after two failures the
/// remaining iterations are assumption-ignored) and
/// `testZombieThreadFailures`.
#[test]
fn test_max_failures_rule() {
    const TOTAL_ITERS: i32 = 500;
    const DESIRED_FAILURES: i32 = TOTAL_ITERS / 10;
    #[derive(Default)]
    struct Nested {
        num_fails: i32,
        num_iters: i32,
    }
    let nested = Class::builder("org.apache.lucene.tests.util.TestMaxFailuresRule$Nested")
        .extends(&abstract_nested_test())
        .constructor(Constructor::no_args(Nested::default))
        .method(
            Method::new("testFailSometimes")
                .annotate(Repeat {
                    iterations: TOTAL_ITERS,
                    use_constant_seed: false,
                })
                .body(|n: &mut Nested| {
                    n.num_iters += 1;
                    let mut fail = LuceneTestCase::random().next_int_bound(5) == 0;
                    if fail {
                        n.num_fails += 1;
                    }
                    if n.num_fails < DESIRED_FAILURES
                        && DESIRED_FAILURES <= TOTAL_ITERS - n.num_iters
                    {
                        fail = true;
                    }
                    Assert::assert_false(fail);
                }),
        )
        .build();
    struct Letters(Mutex<(char, String)>);
    impl RunListener for Letters {
        fn test_started(&self, _d: &Arc<Description>) {
            self.0.lock().unwrap().0 = 'S';
        }
        fn test_assumption_failure(&self, _f: &Failure) {
            self.0.lock().unwrap().0 = 'A';
        }
        fn test_failure(&self, _f: &Failure) {
            self.0.lock().unwrap().0 = 'F';
        }
        fn test_finished(&self, _d: &Arc<Description>) {
            let mut s = self.0.lock().unwrap();
            let c = s.0;
            s.1.push(c);
        }
    }
    run_suite(
        with_nested_tests("org.apache.lucene.tests.util.TestMaxFailuresRule", true)
            .method(test("testMaxFailures", move || {
                LuceneTestCase::replace_max_failure_rule(Arc::new(
                    TestRuleIgnoreAfterMaxFailures::new(2),
                ));
                let letters = Arc::new(Letters(Mutex::new((' ', String::new()))));
                let mut core = JUnitCore::new();
                core.add_listener(Arc::clone(&letters) as Arc<dyn RunListener>);
                let result = core.run(&Request::a_class(&nested));
                Assert::assert_equals(500, result.run_count());
                Assert::assert_equals(0, result.ignore_count());
                Assert::assert_equals(2, result.failure_count());
                let results = letters.0.lock().unwrap().1.clone();
                // `(S*F){2}A+`
                let mut rest = results.as_str();
                let mut ok = true;
                for _ in 0..2 {
                    rest = rest.trim_start_matches('S');
                    match rest.strip_prefix('F') {
                        Some(r) => rest = r,
                        None => ok = false,
                    }
                }
                ok &= !rest.is_empty() && rest.chars().all(|c| c == 'A');
                Assert::assert_true_msg(&results, ok);
            }))
            .method(test("testZombieThreadFailures", zombie_thread_failures)),
    );
}

/// `TestSysoutsLimits.ParentNestedTest` and its subclass `name`, writing
/// with `body` under `limit`.
fn sysouts_nested(name: &str, limit: Limit, body: impl Fn() + Send + Sync + 'static) -> Arc<Class> {
    static PARENT: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.util.TestSysoutsLimits$ParentNestedTest")
            .extends(&RandomizedTest::class())
            .annotate(Limit {
                bytes: TestRuleLimitSysouts::DEFAULT_LIMIT,
                hard_limit: TestRuleLimitSysouts::DEFAULT_HARD_LIMIT,
            })
            .annotate(randomizedtesting::jdk::reflect::Marker(&NESTED_TEST_SUITE))
            .field(
                Field::new_static("classRules", "org.junit.rules.TestRule", || {
                    let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
                    Value::TestRule(Arc::new(
                        RuleChain::outer_rule(Arc::new(TestRuleIgnoreTestSuites))
                            .around(Arc::new(TestRuleMarkFailureRule(Arc::clone(&marker))))
                            .around(Arc::new(TestRuleAdapterRule(Arc::new(
                                TestRuleLimitSysouts::new(marker),
                            )))),
                    ) as Arc<dyn TestRule>)
                })
                .annotate(ClassRule),
            )
            .method(
                Method::new("onlyWhenNested")
                    .annotate(BeforeClass)
                    .static_body(|| {
                        randomizedtesting::junit::Assume::assume_true_msg(
                            "Only runs when nested",
                            TestRuleIgnoreTestSuites::is_running_nested(),
                        );
                    }),
            )
            .build()
    });
    Class::builder(name)
        .extends(&PARENT)
        .annotate(limit)
        .constructor(Constructor::no_args(|| ()))
        .method(test("testWrite", body))
        .build()
}

fn failure_messages(result: &JUnitResult) -> String {
    result
        .failures()
        .iter()
        .map(|f| f.message().unwrap_or("null").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `TestSysoutsLimits` (`super(false)`: the streams are not captured).
#[test]
fn test_sysouts_limits() {
    let over_soft = sysouts_nested(
        "org.apache.lucene.tests.util.TestSysoutsLimits$OverSoftLimit",
        Limit::bytes(10),
        || system::out_print(&RandomizedTest::random_ascii_letters_of_length(10)),
    );
    let under = sysouts_nested(
        "org.apache.lucene.tests.util.TestSysoutsLimits$UnderLimit",
        Limit::bytes(10),
        || system::out_print(&RandomizedTest::random_ascii_letters_of_length(9)),
    );
    let over_hard = sysouts_nested(
        "org.apache.lucene.tests.util.TestSysoutsLimits$OverHardLimit",
        Limit {
            bytes: 10,
            hard_limit: 20,
        },
        || {
            system::out_print("1234567890");
            system::out_print("-marker1-");
            system::out_print("-marker2-");
            system::out_print("-marker3-");
            system::out_print("-marker4-");
        },
    );
    run_outer(
        with_nested_tests("org.apache.lucene.tests.util.TestSysoutsLimits", false)
            .method(test("testOverSoftLimit", move || {
                let msg = failure_messages(&run_classes(&over_soft));
                Assert::assert_true_msg(&msg, msg.contains("The test or suite printed 10 bytes"));
            }))
            .method(test("testUnderLimit", move || {
                let msg = failure_messages(&run_classes(&under));
                Assert::assert_true_msg(&msg, msg.is_empty());
            }))
            .method(test("OverHardLimit", move || {
                let msg = failure_messages(&run_classes(&over_hard));
                Assert::assert_true_msg(&msg, msg.contains("Hard limit was enforced"));
                Assert::assert_true_msg(&msg, msg.contains("The test or suite printed 46 bytes"));
            })),
    );
}

// ---------------------------------------------------------------------------
// Task #359 part 2.
// ---------------------------------------------------------------------------

/// Runs a suite, one at a time, and asserts it reports what Lucene's class
/// reported on the JVM (`framework_tests.tsv`).
fn run_suite(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    oracle_support::assert_framework_outcomes(&builder.build());
}

/// `java.util.concurrent.CountDownLatch`, as far as the tests use it.
struct CountDownLatch(Mutex<i64>, std::sync::Condvar);

impl CountDownLatch {
    fn new(count: i64) -> CountDownLatch {
        CountDownLatch(Mutex::new(count), std::sync::Condvar::new())
    }

    fn count_down(&self) {
        let mut c = self.0.lock().unwrap();
        if *c > 0 {
            *c -= 1;
            if *c == 0 {
                self.1.notify_all();
            }
        }
    }

    fn await_zero(&self) {
        let mut c = self.0.lock().unwrap();
        while *c > 0 {
            c = self.1.wait(c).unwrap();
        }
    }
}

/// `new Thread() { run() { throw new RuntimeException(message); } }`,
/// started and joined.
fn throw_in_thread(message: &'static str) {
    let t = Thread::unnamed(move || exception::runtime(message));
    t.start();
    // `t.join()`: the uncaught exception went to the thread's handler.
    let _ = t.join();
}

/// `TestMaxFailuresRule.Nested2.die`, `zombie` and `testNum`.
static NESTED2_DIE: Mutex<Option<Arc<CountDownLatch>>> = Mutex::new(None);
static NESTED2_ZOMBIE: Mutex<Option<Thread>> = Mutex::new(None);
static NESTED2_TEST_NUM: AtomicI32 = AtomicI32::new(0);

/// `TestMaxFailuresRule.Nested2`: leaves a zombie thread in its second
/// iteration (`@ThreadLeakZombies(IGNORE_REMAINING_TESTS)`,
/// `@ThreadLeakAction(WARN)`, `@ThreadLeakScope(TEST)`,
/// `@ThreadLeakLingering(linger = 500)`).
fn max_failures_nested2() -> Arc<Class> {
    const TOTAL_ITERS: i32 = 10;
    Class::builder("org.apache.lucene.tests.util.TestMaxFailuresRule$Nested2")
        .extends(&abstract_nested_test())
        .annotate(ThreadLeakZombies {
            value: Consequence::IgnoreRemainingTests,
        })
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn],
        })
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(ThreadLeakLingering { linger: 500 })
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("setup").annotate(BeforeClass).static_body(|| {
            // `assert zombie == null;`
            if NESTED2_ZOMBIE.lock().unwrap().is_some() {
                exception::throw(exception::ASSERTION_ERROR, "null");
            }
            *NESTED2_DIE.lock().unwrap() = Some(Arc::new(CountDownLatch::new(1)));
            NESTED2_TEST_NUM.store(0, Ordering::SeqCst);
        }))
        .method(
            Method::new("testLeaveZombie")
                .annotate(Repeat {
                    iterations: TOTAL_ITERS,
                    use_constant_seed: false,
                })
                .body_any(|| {
                    if NESTED2_TEST_NUM.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
                        let die = Arc::clone(NESTED2_DIE.lock().unwrap().as_ref().unwrap());
                        let zombie = Thread::unnamed(move || {
                            // `while (true) { try { die.await(); return; } catch
                            // (Exception ignored) {} }`
                            die.await_zero();
                        });
                        zombie.start();
                        *NESTED2_ZOMBIE.lock().unwrap() = Some(zombie);
                    }
                }),
        )
        .build()
}

/// `TestMaxFailuresRule.testZombieThreadFailures`.
fn zombie_thread_failures() {
    const TOTAL_ITERS: usize = 10;
    LuceneTestCase::replace_max_failure_rule(Arc::new(TestRuleIgnoreAfterMaxFailures::new(1)));
    struct Letters(Mutex<(char, String)>);
    impl RunListener for Letters {
        fn test_started(&self, _d: &Arc<Description>) {
            self.0.lock().unwrap().0 = 'S';
        }
        fn test_assumption_failure(&self, _f: &Failure) {
            self.0.lock().unwrap().0 = 'A';
        }
        fn test_failure(&self, f: &Failure) {
            self.0.lock().unwrap().0 = 'F';
            system::out_println(f.message().unwrap_or("null"));
        }
        fn test_finished(&self, _d: &Arc<Description>) {
            let mut s = self.0.lock().unwrap();
            let c = s.0;
            s.1.push(c);
        }
    }
    let letters = Arc::new(Letters(Mutex::new((' ', String::new()))));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&letters) as Arc<dyn RunListener>);
    let result = core.run(&Request::a_class(&max_failures_nested2()));
    if let Some(die) = NESTED2_DIE.lock().unwrap().as_ref() {
        die.count_down();
        if let Some(zombie) = NESTED2_ZOMBIE.lock().unwrap().as_ref() {
            let _ = zombie.join();
        }
    }
    let results = letters.0.lock().unwrap().1.clone();
    prev_sys_out_println(&results);
    Assert::assert_equals(TOTAL_ITERS, result.run_count());
    Assert::assert_equals_msg(Some(&results), "SFAAAAAAAA", results.as_str());
}

/// `TestExceptionInBeforeClassHooks`: an uncaught exception of a thread
/// started in `@BeforeClass`, in a test or in `@Before` fails the test.
#[test]
fn test_exception_in_before_class_hooks() {
    let nested1 =
        Class::builder("org.apache.lucene.tests.util.TestExceptionInBeforeClassHooks$Nested1")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("beforeClass")
                    .annotate(BeforeClass)
                    .throws(&["java.lang.Exception"])
                    .static_body(|| throw_in_thread("foobar")),
            )
            .method(Method::new("test").body_any(|| {}))
            .build();
    let nested2 =
        Class::builder("org.apache.lucene.tests.util.TestExceptionInBeforeClassHooks$Nested2")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("test1")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| throw_in_thread("foobar1")),
            )
            .method(
                Method::new("test2")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| throw_in_thread("foobar2")),
            )
            .method(
                Method::new("test3")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| throw_in_thread("foobar3")),
            )
            .build();
    let nested3 =
        Class::builder("org.apache.lucene.tests.util.TestExceptionInBeforeClassHooks$Nested3")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("runBeforeTest")
                    .annotate(Before)
                    .throws(&["java.lang.Exception"])
                    .body_any(|| throw_in_thread("foobar")),
            )
            .method(
                Method::new("test1")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {}),
            )
            .build();
    run_suite(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestExceptionInBeforeClassHooks",
            true,
        )
        .method(test("testExceptionInBeforeClassFailsTheTest", move || {
            let run_classes = run_classes(&nested1);
            assert_failure_count(1, &run_classes);
            Assert::assert_equals(1, run_classes.run_count());
            Assert::assert_true(run_classes.failures()[0].trace().contains("foobar"));
        }))
        .method(test("testExceptionWithinTestFailsTheTest", move || {
            let run_classes = run_classes(&nested2);
            assert_failure_count(3, &run_classes);
            Assert::assert_equals(3, run_classes.run_count());
            // `Pattern.compile("foobar[0-9]+").matcher(f.getTrace())`
            let mut foobars: Vec<String> = Vec::new();
            for f in run_classes.failures() {
                let trace = f.trace();
                let mut rest = trace.as_str();
                while let Some(at) = rest.find("foobar") {
                    let tail = &rest[at + "foobar".len()..];
                    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
                    if !digits.is_empty() {
                        foobars.push(format!("foobar{digits}"));
                    }
                    rest = &tail[digits.len()..];
                }
            }
            foobars.sort();
            Assert::assert_equals(
                "[foobar1, foobar2, foobar3]",
                format!("[{}]", foobars.join(", ")).as_str(),
            );
        }))
        .method(test("testExceptionWithinBefore", move || {
            let run_classes = run_classes(&nested3);
            assert_failure_count(1, &run_classes);
            Assert::assert_equals(1, run_classes.run_count());
            Assert::assert_true(run_classes.failures()[0].trace().contains("foobar"));
        })),
    );
}

/// `TestExpectThrows.HuperDuperException extends IOException`.
const HUPER_DUPER: &str = "org.apache.lucene.tests.util.TestExpectThrows$HuperDuperException";

/// Runs `body` and returns what it threw, as a Java `catch (T e)` of the
/// class `catch_class`; anything else propagates.
fn catch_throwable(catch_class: &str, body: impl FnOnce()) -> Option<Throwable> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(()) => None,
        Err(payload) => {
            let t = Throwable::from_panic(payload);
            if t.is_instance(catch_class) {
                Some(t)
            } else {
                t.throw()
            }
        }
    }
}

/// `TestExpectThrows`: `expectThrows` returns the expected exception, fails
/// when nothing is thrown, and lets assertions and assumptions through
/// unless they are the expected type.
#[test]
fn test_expect_throws() {
    exception::register_class(HUPER_DUPER, exception::IO_EXCEPTION);
    let assumption = randomizedtesting::junit::ASSUMPTION_VIOLATED_INTERNAL;
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestExpectThrows")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(Method::new("testPass").body_any(|| {
                let ran = AtomicBool::new(false);
                let returned = LuceneTestCase::expect_throws(exception::IO_EXCEPTION, || {
                    ran.swap(true, Ordering::SeqCst);
                    Throwable::with_message(HUPER_DUPER, None).throw()
                });
                Assert::assert_true(ran.load(Ordering::SeqCst));
                Assert::assert_equals(HUPER_DUPER, returned.class_name());
            }))
            .method(Method::new("testFail").body_any(|| {
                let ran = AtomicBool::new(false);
                let caught = catch_throwable(exception::ASSERTION_ERROR, || {
                    LuceneTestCase::expect_throws(exception::IO_EXCEPTION, || {
                        ran.swap(true, Ordering::SeqCst);
                        Ok(())
                    });
                    Assert::fail("must not complete");
                });
                Assert::assert_true(ran.load(Ordering::SeqCst));
                let caught = caught.unwrap_or_else(|| Assert::fail("caught is null"));
                Assert::assert_equals(
                    "Expected exception IOException but no exception was thrown",
                    caught.message().unwrap_or("null"),
                );
            }))
            .method(Method::new("testNestedFail").body_any(|| {
                let ran = AtomicBool::new(false);
                let caught = catch_throwable(exception::ASSERTION_ERROR, || {
                    LuceneTestCase::expect_throws(exception::IO_EXCEPTION, || {
                        ran.swap(true, Ordering::SeqCst);
                        Assert::fail("this failure should propogate");
                    });
                    Assert::fail("must not complete");
                });
                Assert::assert_true(ran.load(Ordering::SeqCst));
                let caught = caught.unwrap_or_else(|| Assert::fail("caught is null"));
                Assert::assert_equals(
                    "this failure should propogate",
                    caught.message().unwrap_or("null"),
                );
            }))
            .method(Method::new("testNestedAssume").body_any(move || {
                let ran = AtomicBool::new(false);
                let caught = catch_throwable(assumption, || {
                    LuceneTestCase::expect_throws(exception::IO_EXCEPTION, || {
                        ran.swap(true, Ordering::SeqCst);
                        LuceneTestCase::assume_true("this assumption should propogate", false);
                        Ok(())
                    });
                    Assert::fail("must not complete");
                });
                Assert::assert_true(ran.load(Ordering::SeqCst));
                let caught = caught.unwrap_or_else(|| Assert::fail("caught is null"));
                Assert::assert_equals(
                    "this assumption should propogate",
                    caught.message().unwrap_or("null"),
                );
            }))
            .method(Method::new("testExpectingNestedFail").body_any(|| {
                let ran = AtomicBool::new(false);
                let mut returned = None;
                if let Some(caught) = catch_throwable(exception::ASSERTION_ERROR, || {
                    returned = Some(LuceneTestCase::expect_throws(
                        exception::ASSERTION_ERROR,
                        || {
                            ran.swap(true, Ordering::SeqCst);
                            Assert::fail("this failure should be returned, not propogated");
                        },
                    ));
                }) {
                    Assert::fail(&format!(
                        "An exception should not have been thrown expected null, but \
                         was:<{caught}>"
                    ));
                }
                Assert::assert_true(ran.load(Ordering::SeqCst));
                let returned = returned.unwrap_or_else(|| Assert::fail("returned is null"));
                Assert::assert_equals(
                    "this failure should be returned, not propogated",
                    returned.message().unwrap_or("null"),
                );
            }))
            .method(Method::new("testExpectingNestedAssume").body_any(move || {
                let ran = AtomicBool::new(false);
                let mut returned = None;
                if let Some(caught) = catch_throwable(assumption, || {
                    returned = Some(LuceneTestCase::expect_throws(assumption, || {
                        ran.swap(true, Ordering::SeqCst);
                        LuceneTestCase::assume_true(
                            "this assumption should be returned, not propogated",
                            false,
                        );
                        Ok(())
                    }));
                }) {
                    Assert::fail(&format!(
                        "An exception should not have been thrown expected null, but \
                         was:<{caught}>"
                    ));
                }
                Assert::assert_true(ran.load(Ordering::SeqCst));
                let returned = returned.unwrap_or_else(|| Assert::fail("returned is null"));
                Assert::assert_equals(
                    "this assumption should be returned, not propogated",
                    returned.message().unwrap_or("null"),
                );
            })),
    );
}

/// Throws a Rucene error out of a test method, as Java's `throws Exception`.
fn rethrow_lucene(e: rucene::error::LuceneError) -> ! {
    let (class, message) =
        rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message(&e);
    Throwable::new(class, message).throw()
}

/// `TestFailIfDirectoryNotClosed`: a directory left open fails the suite.
#[test]
fn test_fail_if_directory_not_closed() {
    let nested1 =
        Class::builder("org.apache.lucene.tests.util.TestFailIfDirectoryNotClosed$Nested1")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testDummy")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        let dir = LuceneTestCase::new_directory();
                        system::out_println(&dir.to_string());
                    }),
            )
            .build();
    run_suite(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestFailIfDirectoryNotClosed",
            true,
        )
        .method(test("testFailIfDirectoryNotClosed", move || {
            let r = run_classes(&nested1);
            RandomizedTest::assume_true_msg(
                "Ignoring nested test, very likely zombie threads present.",
                r.ignore_count() == 0,
            );
            assert_failure_count(1, &r);
            Assert::assert_true(
                r.failures()[0]
                    .to_string()
                    .contains("Resource in scope SUITE failed to close"),
            );
        })),
    );
}

/// `TestFailIfUnreferencedFiles` (LUCENE-4456): a file no commit references
/// fails `MockDirectoryWrapper.close()`.
#[test]
fn test_fail_if_unreferenced_files() {
    let nested1 =
        Class::builder("org.apache.lucene.tests.util.TestFailIfUnreferencedFiles$Nested1")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testDummy")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        use rucene::document::Document;
                        use rucene::index::index_writer::IndexWriter;
                        use rucene::index::index_writer_config::IndexWriterConfig;
                        use rucene::store::{Directory, IndexOutput, DEFAULT_IO_CONTEXT};
                        let dir = LuceneTestCase::new_mock_directory();
                        dir.set_assert_no_unrefenced_files_on_close(true);
                        let iw = IndexWriter::new(
                            dir.as_directory(),
                            IndexWriterConfig::with_analyzer(None),
                        )
                        .unwrap_or_else(|e| rethrow_lucene(e));
                        iw.add_document(&Document::new())
                            .unwrap_or_else(|e| rethrow_lucene(e));
                        iw.close().unwrap_or_else(|e| rethrow_lucene(e));
                        let mut output = dir
                            .create_output("_hello.world", &*DEFAULT_IO_CONTEXT)
                            .unwrap_or_else(|e| rethrow_lucene(e));
                        output
                            .write_string("i am unreferenced!")
                            .unwrap_or_else(|e| rethrow_lucene(e));
                        IndexOutput::close(&mut *output).unwrap_or_else(|e| rethrow_lucene(e));
                        dir.sync(&["_hello.world".to_string()])
                            .unwrap_or_else(|e| rethrow_lucene(e));
                        dir.close().unwrap_or_else(|e| rethrow_lucene(e));
                    }),
            )
            .build();
    run_suite(
        with_nested_tests(
            "org.apache.lucene.tests.util.TestFailIfUnreferencedFiles",
            true,
        )
        .method(test("testFailIfUnreferencedFiles", move || {
            let r = run_classes(&nested1);
            RandomizedTest::assume_true_msg(
                "Ignoring nested test, very likely zombie threads present.",
                r.ignore_count() == 0,
            );
            // We are suppressing output anyway so dump the failures.
            for f in r.failures() {
                system::out_println(&f.trace());
            }
            Assert::assert_equals_msg(Some("Expected exactly one failure."), 1, r.failure_count());
            Assert::assert_true_msg(
                "Expected unreferenced files assertion.",
                r.failures()[0].trace().contains("unreferenced files:"),
            );
        })),
    );
}

/// `TestFloatingPointUlpEquality.testDoubleEqualsWithAllowedUlps()`
/// (`public static`).
fn test_double_equals_with_allowed_ulps() {
    use rucene_test_framework::util::TestUtil;
    let d = |x: f64, y: f64, u: i32| TestUtil::double_ulp_equals(x, y, u);
    let next_up = |v: f64| randomizedtesting::jdk::math::next_up(v);
    let next_down = |v: f64| randomizedtesting::jdk::math::next_down(v);
    let ulp1 = f64::EPSILON; // `Math.ulp(1d)`
    Assert::assert_true(d(0.0, -0.0, 1));
    Assert::assert_true(d(f64::from_bits(1), -0.0, 1));
    Assert::assert_false(d(f64::from_bits(1), -f64::from_bits(1), 1));

    Assert::assert_true(d(1.0, 1.0 + ulp1, 1));
    Assert::assert_false(d(1.0, 1.0 + 2.0 * ulp1, 1));

    for value in [153.0, -128.0, 0.0, 1.0] {
        Assert::assert_true(d(value, value, 1));
        Assert::assert_true(d(value, next_up(value), 1));
        Assert::assert_false(d(value, next_up(next_up(value)), 1));
        Assert::assert_true(d(value, next_down(value), 1));
        Assert::assert_false(d(value, next_down(next_down(value)), 1));
        Assert::assert_false(d(value, value, -1));
        Assert::assert_false(d(value, next_up(value), 0));
        Assert::assert_true(d(value, next_up(next_up(value)), 2));
        Assert::assert_true(d(value, next_down(next_down(value)), 2));
    }

    Assert::assert_true(d(f64::INFINITY, f64::INFINITY, 1));
    Assert::assert_true(d(f64::MAX, f64::INFINITY, 1));

    Assert::assert_true(d(f64::NEG_INFINITY, f64::NEG_INFINITY, 1));
    Assert::assert_true(d(-f64::MAX, f64::NEG_INFINITY, 1));

    Assert::assert_false(d(f64::NAN, f64::NAN, 1));
    Assert::assert_false(d(f64::NAN, f64::NAN, 0));
    Assert::assert_false(d(f64::NAN, 0.0, 0));
    Assert::assert_false(d(0.0, f64::NAN, 0));
    Assert::assert_false(d(f64::NAN, f64::INFINITY, 0));
    Assert::assert_false(d(f64::NAN, f64::NEG_INFINITY, 0));

    // Create a NaN representation 1 ulp above infinity.
    let nan = f64::from_bits(f64::INFINITY.to_bits() + 1);
    Assert::assert_false(d(nan, f64::INFINITY, 1));
    Assert::assert_false(d(f64::INFINITY, nan, 1));

    Assert::assert_false(d(f64::NEG_INFINITY, f64::INFINITY, i32::MAX));
    Assert::assert_false(d(0.0, f64::MAX, i32::MAX));
    // Here: f == 5.304989477E-315;
    let f = f64::from_bits(1u64 << 30);
    Assert::assert_false(d(-f, f, i32::MAX));
    Assert::assert_true(d(-f, next_down(f), i32::MAX));
    Assert::assert_true(d(next_up(-f), f, i32::MAX));
    // Maximum distance between same sign numbers.
    let f2 = f64::from_bits((1u64 << 30) + i32::MAX as u64);
    Assert::assert_true(d(f, f2, i32::MAX));
    Assert::assert_false(d(f, next_up(f2), i32::MAX));
    Assert::assert_false(d(next_down(f), f2, i32::MAX));
}

/// `TestFloatingPointUlpEquality.testFloatEqualsWithAllowedUlps()`
/// (`public static`).
fn test_float_equals_with_allowed_ulps() {
    use rucene_test_framework::util::TestUtil;
    let fe = |x: f32, y: f32, u: i16| TestUtil::float_ulp_equals(x, y, u);
    let next_up = |v: f32| randomizedtesting::jdk::math::next_up_f32(v);
    let next_down = |v: f32| randomizedtesting::jdk::math::next_down_f32(v);
    let ulp1 = f32::EPSILON; // `Math.ulp(1f)`
    Assert::assert_true(fe(0.0, -0.0, 1));
    Assert::assert_true(fe(f32::from_bits(1), -0.0, 1));
    Assert::assert_false(fe(f32::from_bits(1), -f32::from_bits(1), 1));

    Assert::assert_true(fe(1.0, 1.0 + ulp1, 1));
    Assert::assert_false(fe(1.0, 1.0 + 2.0 * ulp1, 1));

    for value in [153.0f32, -128.0, 0.0, 1.0] {
        Assert::assert_true(fe(value, value, 1));
        Assert::assert_true(fe(value, next_up(value), 1));
        Assert::assert_false(fe(value, next_up(next_up(value)), 1));
        Assert::assert_true(fe(value, next_down(value), 1));
        Assert::assert_false(fe(value, next_down(next_down(value)), 1));
        Assert::assert_false(fe(value, value, -1));
        Assert::assert_false(fe(value, next_up(value), 0));
        Assert::assert_true(fe(value, next_up(next_up(value)), 2));
        Assert::assert_true(fe(value, next_down(next_down(value)), 2));
    }

    Assert::assert_true(fe(f32::INFINITY, f32::INFINITY, 1));
    Assert::assert_true(fe(f32::MAX, f32::INFINITY, 1));

    Assert::assert_true(fe(f32::NEG_INFINITY, f32::NEG_INFINITY, 1));
    Assert::assert_true(fe(-f32::MAX, f32::NEG_INFINITY, 1));

    Assert::assert_false(fe(f32::NAN, f32::NAN, 1));
    Assert::assert_false(fe(f32::NAN, f32::NAN, 0));
    Assert::assert_false(fe(f32::NAN, 0.0, 0));
    Assert::assert_false(fe(0.0, f32::NAN, 0));
    Assert::assert_false(fe(f32::NAN, f32::INFINITY, 0));
    Assert::assert_false(fe(f32::NAN, f32::NEG_INFINITY, 0));

    Assert::assert_false(fe(f32::NEG_INFINITY, f32::INFINITY, 32767));
    // The 31-bit integer specification of the max positive ULP allows an
    // extremely large range of a 23-bit mantissa and 8-bit exponent
    // (`TestFloatingPointUlpEquality.java:122`: false on the JVM too).
    Assert::assert_true(fe(0.0, f32::MAX, 32767));
    // Here: f == 2;
    let f = f32::from_bits(1 << 30);
    Assert::assert_false(fe(-f, f, 32767));
    Assert::assert_true(fe(-f, next_down(f), 32767));
    Assert::assert_true(fe(next_up(-f), f, 32767));
    let f2 = f32::from_bits(i32::MAX as u32);
    Assert::assert_equals_double_msg(None, f64::NAN, f64::from(f2), 0.0);
    Assert::assert_false(fe(f2, f32::MAX, 32767));
    Assert::assert_false(fe(f2, 0.0, 32767));
}

/// `TestFloatingPointUlpEquality`: its two test methods are `public static`,
/// which `LuceneTestCase`'s JUnit 3 provider skips, so the suite runs no
/// test on the JVM (`run=0`), and neither does the port.
#[test]
fn test_floating_point_ulp_equality() {
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestFloatingPointUlpEquality")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testDoubleEqualsWithAllowedUlps")
                    .static_body(test_double_equals_with_allowed_ulps),
            )
            .method(
                Method::new("testFloatEqualsWithAllowedUlps")
                    .static_body(test_float_equals_with_allowed_ulps),
            ),
    );
}

/// The two static bodies of `TestFloatingPointUlpEquality`, called directly
/// as `FrameworkTestsOracle`'s companion run called them by reflection on
/// the JVM: the double one passes; the float one fails with a bare
/// `AssertionError` at `assertTrue(floatUlpEquals(0, Float.MAX_VALUE,
/// (short) 32767))` (line 122), a defect of Lucene's test that its being
/// static hides.
#[test]
fn test_floating_point_ulp_equality_static_bodies() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    test_double_equals_with_allowed_ulps();
    let failure = catch_unwind(test_float_equals_with_allowed_ulps)
        .map(|()| None)
        .unwrap_or_else(|p| Some(Throwable::from_panic(p)))
        .expect("the float body fails, as on the JVM");
    assert_eq!(failure.class_name(), exception::ASSERTION_ERROR);
    assert_eq!(failure.message(), None);
    // The failing assertion is line 122's: every earlier one holds.
    assert!(!rucene_test_framework::util::TestUtil::float_ulp_equals(
        0.0,
        f32::MAX,
        32767
    ));
}

/// `@TestGroup(enabled = false)` group annotations of
/// `TestGroupFiltering`: `@Foo`, `@Bar` and `@Jira(bug)`.
static FOO: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.TestGroupFiltering$Foo",
    simple_name: "Foo",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};
static BAR: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.TestGroupFiltering$Bar",
    simple_name: "Bar",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};
static JIRA: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.TestGroupFiltering$Jira",
    simple_name: "Jira",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};

/// `@Jira(bug = ...)`.
#[derive(Debug)]
struct Jira {
    bug: &'static str,
}

impl JavaAnnotation for Jira {
    fn annotation_type(&self) -> &'static AnnotationType {
        &JIRA
    }
    fn to_java_string(&self) -> String {
        format!(
            "@{}(bug={})",
            JIRA.canonical_name(),
            randomizedtesting::jdk::reflect::quote_member(self.bug)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `TestGroupFiltering`: every test is in a disabled group.
#[test]
fn test_group_filtering() {
    use randomizedtesting::jdk::reflect::Marker;
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestGroupFiltering")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testFoo")
                    .annotate(Marker(&FOO))
                    .body_any(|| {}),
            )
            .method(
                Method::new("testFooBar")
                    .annotate(Marker(&FOO))
                    .annotate(Marker(&BAR))
                    .body_any(|| {}),
            )
            .method(
                Method::new("testBar")
                    .annotate(Marker(&BAR))
                    .body_any(|| {}),
            )
            .method(
                Method::new("testJira")
                    .annotate(Jira {
                        bug: "JIRA bug reference",
                    })
                    .body_any(|| {}),
            ),
    );
}

/// `TestJvmInfo extends RandomizedTest`: echoes the runtime's properties.
#[test]
fn test_jvm_info() {
    let p = |k: &str| system::get_property(k).unwrap_or_else(|| "null".to_string());
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestJvmInfo")
            .extends(&RandomizedTest::class())
            .constructor(Constructor::no_args(|| ()))
            .method(test("testEchoJvmInfo", move || {
                system::out_print(&format!(
                    "This test runs with Java {} ({}, {} {}).\n",
                    p("java.version"),
                    p("java.vendor"),
                    p("java.vm.name"),
                    p("java.vm.version")
                ));
            })),
    );
}

/// `TestLineFileDocs.testDateFieldNormalization`.
#[test]
fn test_line_file_docs() {
    use rucene_test_framework::util::line_file_docs::{
        date_field_value_to_local_date_time as apply, LocalDateTime,
    };
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestLineFileDocs")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(Method::new("testDateFieldNormalization").body_any(|| {
                // europarl corpus uses this data format.
                Assert::assert_equals(
                    format!("{:?}", LocalDateTime::of(2023, 2, 23, 0, 0, 0)),
                    format!("{:?}", apply("2023-02-23").unwrap()),
                );
                // enwiki uses this data format.
                Assert::assert_equals(
                    format!("{:?}", LocalDateTime::of(2010, 1, 12, 12, 32, 45)),
                    format!("{:?}", apply("12-JAN-2010 12:32:45.000").unwrap()),
                );
            })),
    );
}

/// `TestRamUsageTesterOnWildAnimals.ListElement`.
struct ListElement {
    next: Option<Box<ListElement>>,
}

impl Drop for ListElement {
    /// Unlinks the chain iteratively (Java's collector needs no stack).
    fn drop(&mut self) {
        let mut next = self.next.take();
        while let Some(mut n) = next {
            next = n.next.take();
        }
    }
}

impl RamObject for ListElement {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.TestRamUsageTesterOnWildAnimals$ListElement"
    }
    fn java_object(&self) -> JavaObject<'_> {
        use rucene::util::RamUsageEstimator as E;
        JavaObject::Object {
            aligned_shallow_size: E::align_object_size(
                E::NUM_BYTES_OBJECT_HEADER + E::NUM_BYTES_OBJECT_REF,
            ),
            fields: vec![("next", self.next.as_deref().map(|n| n as &dyn RamObject))],
        }
    }
}

/// `TestRamUsageTesterOnWildAnimals.testOverflowMaxChainLength`.
///
/// **Divergence.** Java catches the `StackOverflowError` a recursive walk
/// would throw and bisects on it; Rust cannot catch a stack overflow (the
/// process aborts), so the port has no `catch` branch: the walk is
/// iterative, as Java's is, and the bisection always moves `lower`.
#[test]
fn test_ram_usage_tester_on_wild_animals() {
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestRamUsageTesterOnWildAnimals")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(Method::new("testOverflowMaxChainLength").body_any(|| {
                let upperlimit = LuceneTestCase::at_least(10000);
                let mut lower = 0;
                let upper = upperlimit;
                while lower + 1 < upper {
                    let mid = (lower + upper) / 2;
                    // `first`, then `mid` more elements linked behind it.
                    let mut first = ListElement { next: None };
                    for _ in 0..mid {
                        first = ListElement {
                            next: Some(Box::new(first)),
                        };
                    }
                    RamUsageTester::ram_used(&first); // cause SOE or pass.
                    lower = mid;
                }
                if lower + 1 < upperlimit {
                    Assert::fail(&format!(
                        "Max object chain length till stack overflow: {lower}"
                    ));
                }
            })),
    );
}

/// `TestSeedFromUncaught`: an uncaught exception's failure names the seed.
#[test]
fn test_seed_from_uncaught() {
    let throw_in_uncaught =
        Class::builder("org.apache.lucene.tests.util.TestSeedFromUncaught$ThrowInUncaught")
            .extends(&abstract_nested_test())
            .constructor(Constructor::no_args(|| ()))
            .method(test("testFoo", || throw_in_thread("foobar")).throws(&["java.lang.Exception"]))
            .build();
    run_suite(
        with_nested_tests("org.apache.lucene.tests.util.TestSeedFromUncaught", true).method(test(
            "testUncaughtDumpsSeed",
            move || {
                let result = run_classes(&throw_in_uncaught);
                assert_failure_count(1, &result);
                let trace = result.failures()[0].trace();
                Assert::assert_true(trace.contains("SeedInfo.seed("));
                Assert::assert_true(trace.contains("foobar"));
            },
        )),
    );
}

/// `TestWorstCaseTestBehavior`: six `@Ignore`d tests of the runner's worst
/// cases; none runs.
#[test]
fn test_worst_case_test_behavior() {
    use randomizedtesting::annotations::Timeout;
    use randomizedtesting::junit::Ignore;
    let ignore = || Ignore {
        value: String::new(),
    };
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestWorstCaseTestBehavior")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testThreadLeak")
                    .annotate(ignore())
                    .body_any(|| {
                        let t = Thread::unnamed(|| {
                            let _ = Thread::try_sleep(10000);
                        });
                        t.start();
                        while !t.is_alive() {
                            std::thread::yield_now();
                        }
                        // once alive, leave it to run outside of the test scope.
                    }),
            )
            .method(
                Method::new("testLaaaaaargeOutput")
                    .annotate(ignore())
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        let message = "I will not OOM on large output";
                        let mut how_much: i64 = 250 * 1024 * 1024;
                        let mut i = 0;
                        while i < how_much {
                            if i > 0 {
                                system::out_print(",\n");
                            }
                            system::out_print(message);
                            how_much -= message.len() as i64; // approximately.
                            i += 1;
                        }
                        system::out_println(".");
                    }),
            )
            .method(
                Method::new("testProgressiveOutput")
                    .annotate(ignore())
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        for i in 0..20 {
                            system::out_println(&format!("Emitting sysout line: {i}"));
                            system::err_println(&format!("Emitting syserr line: {i}"));
                            RandomizedTest::sleep(1000);
                        }
                    }),
            )
            .method(
                Method::new("testUncaughtException")
                    .annotate(ignore())
                    .throws(&["java.lang.Exception"])
                    .body_any(|| throw_in_thread("foobar")),
            )
            .method(
                Method::new("testTimeout")
                    .annotate(ignore())
                    .annotate(Timeout { millis: 500 })
                    .throws(&["java.lang.Exception"])
                    .body_any(|| Thread::sleep(5000)),
            )
            .method(
                Method::new("testZombie")
                    .annotate(ignore())
                    .annotate(Timeout { millis: 1000 })
                    .throws(&["java.lang.Exception"])
                    .body_any(|| loop {
                        let _ = Thread::try_sleep(1000);
                    }),
            ),
    );
}

/// `TestRunWithRestrictedPermissions`: every test needs a security manager,
/// which JDK 25 does not have and Rust has no counterpart of; each one is
/// skipped by `runWithRestrictedPermissions`'s assumption, on the JVM as in
/// the port (whose `run_with_restricted_permissions` declares it).
#[test]
fn test_run_with_restricted_permissions() {
    /// `doSomeForbiddenStuff()`.
    fn do_some_forbidden_stuff() {
        LuceneTestCase::create_temp_dir_prefix("cannot_create_temp_folder");
    }
    /// `doSomeCompletelyForbiddenStuff()`: "something like this should not
    /// never pass!!".
    fn do_some_completely_forbidden_stuff() {
        use rucene::store::java_nio_file::{default_file_system, Files, Path};
        let _ = Files::create_file(&Path::new(default_file_system(), "denied"));
    }
    const SECURITY_EXCEPTION: &str = "java.lang.SecurityException";
    run_suite(
        Class::builder("org.apache.lucene.tests.util.TestRunWithRestrictedPermissions")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testDefaultsPass")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        // `new AllPermission()`: Rust has no permissions.
                        LuceneTestCase::run_with_restricted_permissions(do_some_forbidden_stuff);
                    }),
            )
            .method(
                Method::new("testNormallyAllowedStuff")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        LuceneTestCase::expect_throws(SECURITY_EXCEPTION, || {
                            LuceneTestCase::run_with_restricted_permissions(
                                do_some_forbidden_stuff,
                            );
                            Ok(())
                        });
                    }),
            )
            .method(
                Method::new("testCompletelyForbidden1")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        LuceneTestCase::expect_throws(SECURITY_EXCEPTION, || {
                            LuceneTestCase::run_with_restricted_permissions(
                                do_some_completely_forbidden_stuff,
                            );
                            Ok(())
                        });
                    }),
            )
            .method(
                Method::new("testCompletelyForbidden2")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        LuceneTestCase::expect_throws(SECURITY_EXCEPTION, || {
                            LuceneTestCase::run_with_restricted_permissions(
                                do_some_completely_forbidden_stuff,
                            );
                            Ok(())
                        });
                    }),
            ),
    );
}

/// `TestTestObjectInputFilterFactory`.
///
/// **Divergence.** The class tests Lucene's `TestObjectInputFilterFactory`,
/// a JVM-wide `ObjectInputFilter` factory for Java object serialization;
/// neither exists in Rust. Its `@BeforeClass` assumption ("Only works when
/// TestObjectInputFilterFactory is installed in JVM") is therefore always
/// false in the port, as it is on a JVM started without the factory (where
/// `FrameworkTestsOracle` ran it): the class is skipped and its four tests
/// are reported ignored. Their bodies (an `ObjectOutputStream` /
/// `ObjectInputStream` round trip of a `HashMap` under a serial filter)
/// cannot be expressed and throw `UnsupportedOperationException`; they are
/// never reached.
#[test]
fn test_test_object_input_filter_factory() {
    fn not_portable() {
        exception::throw(
            "java.lang.UnsupportedOperationException",
            "Java object serialization has no Rust counterpart",
        )
    }
    let mut builder =
        Class::builder("org.apache.lucene.tests.util.TestTestObjectInputFilterFactory")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("beforeClass")
                    .annotate(BeforeClass)
                    .static_body(|| {
                        // `getSerialFilterFactory() instanceof TestObjectInputFilterFactory`
                        let installed = false;
                        LuceneTestCase::assume_true(
                            "Only works when TestObjectInputFilterFactory is installed in JVM",
                            installed,
                        );
                    }),
            );
    for name in [
        "testDeserializationWithoutFilter",
        "testDeserializationWithAllowFilter",
        "testDeserializationWithDenyFilter",
        "testBadCodeJustSettingNullFilter",
    ] {
        builder = builder.method(
            Method::new(name)
                .throws(&["java.lang.Exception"])
                .body_any(not_portable),
        );
    }
    run_suite(builder);
}
