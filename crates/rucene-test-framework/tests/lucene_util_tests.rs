//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/util/`
//! that exercise `LuceneTestCase`'s rule chains (task #359, util part):
//! `TestPleaseFail`, `TestJUnitRuleOrder`, `TestSetupTeardownChaining`,
//! `TestBeforeAfterOverrides`, `TestCodecReported`, `TestReproduceMessage`
//! (with `SorePoint` and `SoreType`), `TestReproduceMessageWithRepeated`,
//! `TestMaxFailuresRule.testMaxFailures` and `TestSysoutsLimits`, each run as
//! a suite by the ported runner, nested suites included.

mod with_nested_tests;

use std::sync::{Arc, Mutex};

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{
    After, AfterClass, Assert, Before, BeforeClass, ClassRule, Description, Failure, JUnitCore,
    JUnitResult, Request, Rule, RuleChain, RunListener, Statement, Test, TestRule,
};
use randomizedtesting::rules::TestRuleAdapterRule;
use randomizedtesting::RandomizedTest;
use rucene_test_framework::util::lucene_test_case::LuceneTestCase;
use rucene_test_framework::util::test_rule_ignore_test_suites::NESTED_TEST_SUITE;
use rucene_test_framework::util::test_rule_limit_sysouts::{Limit, TestRuleLimitSysouts};
use rucene_test_framework::util::test_rule_mark_failure::{
    TestRuleMarkFailure, TestRuleMarkFailureRule,
};
use rucene_test_framework::util::{TestRuleIgnoreAfterMaxFailures, TestRuleIgnoreTestSuites};
use with_nested_tests::{
    abstract_nested_test, get_sys_err, is_running_nested, with_nested_tests, SUITES,
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
                    .annotate(Before)
                    .body_any(|| system::out_println("Hello.")),
            )
            .method(test("testMe", || {}))
            .build();
    let teardown = Class::builder(
        "org.apache.lucene.tests.util.TestSetupTeardownChaining$NestedTeardownChain",
    )
    .extends(&abstract_nested_test())
    .constructor(Constructor::no_args(|| ()))
    .method(Method::new("tearDown").annotate(After).body_any(|| {}))
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

/// `TestMaxFailuresRule.testMaxFailures`: after two failures the remaining
/// iterations are assumption-ignored.
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
    run_outer(
        with_nested_tests("org.apache.lucene.tests.util.TestMaxFailuresRule", true).method(test(
            "testMaxFailures",
            move || {
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
            },
        )),
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
