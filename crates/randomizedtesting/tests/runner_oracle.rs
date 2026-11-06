//! Runner oracle parity (`harness = false`): the suites of
//! `tests/fixtures/oracle/src/com/carrotsearch/randomizedtesting/RunnerOracle.java`,
//! declared in Rust and run by the ported `RandomizedRunner` through the
//! ported `JUnitCore`, must print the trace the Java suites print under
//! randomizedtesting 2.8.1 (`tests/fixtures/runner_traces.tsv` and
//! `runner_traces_asserts.tsv`).
//!
//! Masked on both sides before comparing: thread ids (`id=N`, `#N`), thread
//! states (`state=…`: the port observes only `NEW`, `RUNNABLE`,
//! `TERMINATED`), stack frames (the port has no Java frames) and the
//! unnamed module's identity hash (`unnamed module @…`).
//!
//! Every suite, hook, rule and test below mirrors one Java declaration of
//! the oracle, with the same name, modifiers, annotations and body.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use randomizedtesting::annotations::{
    Action, ClassRef, InstanceProviderType, Listeners, Nightly, ParametersFactory, Repeat, Scope,
    Seed, SeedDecoratorClass, SeedDecorators, Seeds, TestCaseInstanceProvider, TestCaseOrdering,
    TestContextRandomSupplier, TestGroup, TestMethodProviders, ThreadLeakAction, ThreadLeakFilters,
    ThreadLeakLingering, ThreadLeakScope, Timeout, TimeoutSuite,
};
use randomizedtesting::jdk::exception::{
    Throwable, ILLEGAL_ARGUMENT_EXCEPTION, ILLEGAL_STATE_EXCEPTION, RUNTIME_EXCEPTION,
};
use randomizedtesting::jdk::logging::{self, Level};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{
    modifier, AnnotationType, Class, ClassBuilder, Constructor, Field, Marker, Method, Value,
};
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{
    After, AfterClass, Assert, Before, BeforeClass, ClassRule, Description, Failure, Ignore,
    JUnitCore, JUnitResult, Request, Rule, RuleChain, RunListener, Statement, Test, TestRule,
};
use randomizedtesting::rules::{
    NoClassHooksShadowingRule, NoInstanceHooksOverridesRule, RequireAssertionsRule,
    StaticFieldsInvariantRule, SystemPropertiesInvariantRule, SystemPropertiesRestoreRule,
    TestRuleAdapter, TestRuleAdapterRule,
};
use randomizedtesting::{
    seed_from_throwable, JUnit3MethodProvider, RandomSupplier, RandomizedContext, RandomizedTest,
    ReproduceErrorMessageBuilder, ThreadFilter,
};

const P: &str = "com.carrotsearch.randomizedtesting.RunnerOracle$";

static OUT: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log(s: impl Into<String>) {
    OUT.lock().unwrap_or_else(|e| e.into_inner()).push(s.into());
}

fn draw() -> String {
    let r = RandomizedContext::current().get_random();
    format!(
        "runner={} int={} long={}",
        RandomizedContext::current().get_runner_seed_as_string(),
        r.next_int(),
        r.next_long()
    )
}

fn thr(t: &Throwable) -> String {
    let mut b = t.to_string().replace('\n', "\\n");
    if let Some(seed) = seed_from_throwable(t) {
        b.push_str(&format!(" [seed {seed}]"));
    }
    let mut c = t.cause();
    while let Some(x) = c {
        b.push_str(&format!(" <- {}", x.to_string().replace('\n', "\\n")));
        c = x.cause();
    }
    b
}

fn tree(d: &Description, indent: &str) {
    log(format!("desc {indent}{}", d.display_name()));
    for c in d.children() {
        tree(&c, &format!("{indent}  "));
    }
}

/// The oracle's `Trace` listener.
struct Trace;

impl RunListener for Trace {
    fn test_run_started(&self, d: &Arc<Description>) {
        log(format!("runStarted tests={}", d.test_count()));
        tree(d, "");
    }
    fn test_run_finished(&self, r: &JUnitResult) {
        log(format!(
            "runFinished run={} fail={} ignore={} assumption={}",
            r.run_count(),
            r.failure_count(),
            r.ignore_count(),
            r.assumption_failure_count()
        ));
    }
    fn test_started(&self, d: &Arc<Description>) {
        log(format!("started {d}"));
    }
    fn test_finished(&self, d: &Arc<Description>) {
        log(format!("finished {d}"));
    }
    fn test_ignored(&self, d: &Arc<Description>) {
        log(format!("ignored {d}"));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        log(format!(
            "assumption {} | {}",
            f.description(),
            thr(f.exception())
        ));
    }
    fn test_failure(&self, f: &Failure) {
        log(format!(
            "failure {} | {}",
            f.description(),
            thr(f.exception())
        ));
        let mut b = ReproduceErrorMessageBuilder::new();
        b.append_all_opts(f.description());
        log(format!("reproduce {}", b.text()));
    }
}

fn scenario(w: &mut String, name: &str, class: &Arc<Class>, props: &[(&str, &str)]) {
    OUT.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let saved: Vec<(String, Option<String>)> = props
        .iter()
        .map(|(k, v)| (k.to_string(), system::set_property(k, v)))
        .collect();
    let mut core = JUnitCore::new();
    core.add_listener(Arc::new(Trace));
    core.run(&Request::a_class(class));
    for (k, v) in saved {
        match v {
            None => {
                system::clear_property(&k);
            }
            Some(v) => {
                system::set_property(&k, &v);
            }
        }
    }
    w.push_str(&format!("=== {name}\n"));
    for s in OUT.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        w.push_str(s);
        w.push('\n');
    }
}

// -----------------------------------------------------------------------------
// Declaration helpers.
// -----------------------------------------------------------------------------

fn cls(name: &str) -> ClassBuilder {
    Class::builder(format!("{P}{name}"))
}

fn unit() -> Constructor {
    Constructor::no_args(|| ())
}

fn test(name: &str, body: impl Fn() + Send + Sync + 'static) -> Method {
    Method::new(name).annotate(Test::new()).body_any(body)
}

fn static_hook(
    name: &str,
    ann: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: impl Fn() + Send + Sync + 'static,
) -> Method {
    Method::new(name).annotate(ann).static_body(body)
}

fn hook(
    name: &str,
    ann: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: impl Fn() + Send + Sync + 'static,
) -> Method {
    Method::new(name).annotate(ann).body_any(body)
}

fn logging_rule(name: &'static str) -> Value {
    Value::TestRule(Arc::new(LoggingRule(name)))
}

fn rule_field(name: &str, make: fn() -> Value) -> Field {
    Field::new_instance::<()>(name, "org.junit.rules.TestRule", move |_| make()).annotate(Rule)
}

fn class_rule_field(name: &str, make: impl Fn() -> Value + Send + Sync + 'static) -> Field {
    Field::new_static(name, "org.junit.rules.TestRule", make).annotate(ClassRule)
}

fn runtime(msg: &str) -> ! {
    Throwable::new(RUNTIME_EXCEPTION, msg).throw()
}

struct LoggingRule(&'static str);

impl TestRule for LoggingRule {
    fn apply(&self, base: Statement, d: &Arc<Description>) -> Statement {
        let name = self.0;
        let d = Arc::clone(d);
        Statement::new(move || {
            log(format!("rule {name} before {}", d.display_name()));
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| base.evaluate()));
            log(format!("rule {name} after"));
            if let Err(p) = r {
                std::panic::resume_unwind(p);
            }
        })
    }
}

// -----------------------------------------------------------------------------
// Hook, rule and seed order.
// -----------------------------------------------------------------------------

static HOOK_SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("HookSuper")
        .constructor(unit())
        .field(class_rule_field("superClassRule", || {
            logging_rule("superClassRule")
        }))
        .field(rule_field("superRule", || logging_rule("superRule")))
        .method(static_hook("beforeClassSuper", BeforeClass, || {
            log("beforeClassSuper")
        }))
        .method(static_hook("afterClassSuper", AfterClass, || {
            log("afterClassSuper")
        }))
        .method(hook("beforeSuper", Before, || log("beforeSuper")))
        .method(hook("afterSuper", After, || log("afterSuper")))
        .build()
});

static HOOK_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("HookSub")
        .extends(&HOOK_SUPER)
        .constructor(Constructor::no_args(|| log("constructor")))
        .field(class_rule_field("classRuleA", || {
            logging_rule("classRuleA")
        }))
        .field(class_rule_field("classRuleB", || {
            logging_rule("classRuleB")
        }))
        .field(rule_field("ruleA", || logging_rule("ruleA")))
        .field(rule_field("ruleB", || logging_rule("ruleB")))
        .field(rule_field("chain", || {
            Value::TestRule(Arc::new(
                RuleChain::outer_rule(Arc::new(LoggingRule("outer")))
                    .around(Arc::new(LoggingRule("inner"))),
            ))
        }))
        .method(static_hook("beforeClass1", BeforeClass, || {
            log(format!("beforeClass1 {}", draw()))
        }))
        .method(static_hook("beforeClass2", BeforeClass, || {
            log("beforeClass2")
        }))
        .method(
            Method::new("beforeClass3")
                .modifiers(modifier::PRIVATE)
                .annotate(BeforeClass)
                .static_body(|| log("beforeClass3")),
        )
        .method(static_hook("afterClass1", AfterClass, || {
            log("afterClass1")
        }))
        .method(static_hook("afterClass2", AfterClass, || {
            log("afterClass2")
        }))
        .method(hook("before1", Before, || log("before1")))
        .method(hook("before2", Before, || log("before2")))
        .method(hook("after1", After, || log("after1")))
        .method(hook("after2", After, || log("after2")))
        .method(test("testA", || {
            log(format!(
                "testA {} thread={}",
                draw(),
                Thread::current().name()
            ))
        }))
        .method(test("testB", || log(format!("testB {}", draw()))))
        .method(test("testC", || {
            log(format!(
                "testC {} method={}",
                draw(),
                RandomizedContext::current()
                    .get_target_method()
                    .unwrap_or_default()
            ))
        }))
        .method(test("testD", || log(format!("testD {}", draw()))))
        .build()
});

static SEEDS1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Seeds1")
        .annotate(Seed::new("DEADBEEF"))
        .constructor(unit())
        .method(test("testPlain", || log(format!("testPlain {}", draw()))))
        .method(
            test("testFixed", || log(format!("testFixed {}", draw()))).annotate(Seed::new("1234")),
        )
        .method(
            test("testSeeds", || log(format!("testSeeds {}", draw()))).annotate(Seeds::new(&[
                "1",
                "2",
                "random",
                "FFFFFFFFFFFFFFFF",
            ])),
        )
        .build()
});

static SEED_CHAIN_ON_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("SeedChainOnClass")
        .annotate(Seed::new("CAFEBABE:12345678"))
        .constructor(unit())
        .method(test("testOne", || log(format!("testOne {}", draw()))))
        .method(test("testTwo", || log(format!("testTwo {}", draw()))))
        .build()
});

static REPEATS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Repeats")
        .constructor(unit())
        .method(
            test("testRepeat", || log(format!("testRepeat {}", draw()))).annotate(Repeat {
                iterations: 3,
                use_constant_seed: false,
            }),
        )
        .method(
            test("testConstant", || log(format!("testConstant {}", draw()))).annotate(Repeat {
                iterations: 2,
                use_constant_seed: true,
            }),
        )
        .method(test("testOnce", || log(format!("testOnce {}", draw()))))
        .build()
});

static REPEATS_ON_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("RepeatsOnClass")
        .annotate(Repeat {
            iterations: 2,
            use_constant_seed: false,
        })
        .constructor(unit())
        .method(test("testX", || log(format!("testX {}", draw()))))
        .method(
            test("testY", || log(format!("testY {}", draw()))).annotate(Repeat {
                iterations: 1,
                use_constant_seed: false,
            }),
        )
        .build()
});

// -----------------------------------------------------------------------------
// Failures, assumptions, ignores, expectations.
// -----------------------------------------------------------------------------

static FAILURES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Failures")
        .constructor(unit())
        .method(test("testAssertion", || {
            log("testAssertion");
            Assert::fail("expected failure")
        }))
        .method(test("testRuntime", || runtime("runtime boom")))
        .method(test("testAssume", || {
            RandomizedTest::assume_true_msg("not this time", false)
        }))
        .method(test("testIgnored", || log("never")).annotate(Ignore::because("ignored for now")))
        .method(
            Method::new("testExpected")
                .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
                .body_any(|| Throwable::new(ILLEGAL_ARGUMENT_EXCEPTION, "ok").throw()),
        )
        .method(
            Method::new("testExpectedMissing")
                .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
                .body_any(|| log("testExpectedMissing")),
        )
        .method(
            Method::new("testExpectedWrong")
                .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
                .body_any(|| Throwable::new(ILLEGAL_STATE_EXCEPTION, "wrong").throw()),
        )
        .method(test("testPass", || log("testPass")))
        .build()
});

static HOOK_FAILURES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("HookFailures")
        .constructor(unit())
        .method(hook("beforeA", Before, || {
            log("beforeA");
            runtime("beforeA failed")
        }))
        .method(hook("beforeB", Before, || {
            log("beforeB");
            runtime("beforeB failed")
        }))
        .method(hook("afterA", After, || {
            log("afterA");
            runtime("afterA failed")
        }))
        .method(hook("afterB", After, || {
            log("afterB");
            runtime("afterB failed")
        }))
        .method(static_hook("afterClass", AfterClass, || {
            log("afterClass");
            runtime("afterClass failed")
        }))
        .method(test("testOne", || log("testOne")))
        .build()
});

static CLASS_ASSUMPTION: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ClassAssumption")
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            log("beforeClass");
            RandomizedTest::assume_true_msg("class-level", false)
        }))
        .method(test("testOne", || log("never")))
        .method(test("testTwo", || log("never")))
        .build()
});

static BEFORE_CLASS_FAILURE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("BeforeClassFailure")
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            Throwable::new(ILLEGAL_STATE_EXCEPTION, "beforeClass failed").throw()
        }))
        .method(static_hook("afterClass", AfterClass, || log("afterClass")))
        .method(test("testOne", || log("never")))
        .build()
});

// -----------------------------------------------------------------------------
// Groups and filters.
// -----------------------------------------------------------------------------

static SLOW: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.RunnerOracle$Slow",
    simple_name: "Slow",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};

static FAST: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.RunnerOracle$Fast",
    simple_name: "Fast",
    test_group: Some(TestGroup {
        name: "fast",
        sys_property: "oracle.fast",
        enabled: true,
    }),
    inherited: true,
};

static GROUPS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Groups")
        .constructor(unit())
        .method(test("testPlain", || log("testPlain")))
        .method(
            test("testNightly", || {
                log(format!(
                    "testNightly nightly={}",
                    RandomizedTest::is_nightly()
                ))
            })
            .annotate(Nightly::default()),
        )
        .method(test("testSlow", || log("testSlow")).annotate(Marker(&SLOW)))
        .method(test("testFast", || log("testFast")).annotate(Marker(&FAST)))
        .method(
            test("testSlowFast", || log("testSlowFast"))
                .annotate(Marker(&SLOW))
                .annotate(Marker(&FAST)),
        )
        .method(test("testOther", || log("testOther")))
        .build()
});

static NIGHTLY_SUITE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("NightlySuite")
        .annotate(Nightly::default())
        .constructor(unit())
        .method(test("testOne", || log("testOne")))
        .build()
});

// -----------------------------------------------------------------------------
// Parameterized suites, providers, ordering, listeners.
// -----------------------------------------------------------------------------

struct ParamsInst {
    value: String,
    number: i32,
}

fn params_args(args: &[Value]) -> ParamsInst {
    let value = match &args[0] {
        Value::Str(s) => s.clone(),
        other => other.to_string(),
    };
    let number = match &args[1] {
        Value::Int(i) => *i,
        _ => 0,
    };
    ParamsInst { value, number }
}

static PARAMS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Params")
        .constructor(Constructor::with_params(
            &[("java.lang.String", Some("value")), ("int", None)],
            |args| {
                let p = params_args(args);
                log(format!("constructor {} {}", p.value, p.number));
                p
            },
        ))
        .method(
            Method::new("parameters")
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory::default())
                .static_value(|_| {
                    let row =
                        |s: &str, i: i32| Value::Array(vec![Value::Str(s.into()), Value::Int(i)]);
                    Value::List(vec![row("a", 1), row("b", 2), row("c", 3), row("a", 1)])
                }),
        )
        .method(
            Method::new("testOne")
                .annotate(Test::new())
                .body(|p: &mut ParamsInst| {
                    log(format!("testOne {} {} {}", p.value, p.number, draw()))
                }),
        )
        .method(
            Method::new("testTwo")
                .annotate(Test::new())
                .annotate(Repeat {
                    iterations: 2,
                    use_constant_seed: false,
                })
                .body(|p: &mut ParamsInst| log(format!("testTwo {} {}", p.value, p.number))),
        )
        .build()
});

static PARAMS_FORMATTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ParamsFormatted")
        .annotate(TestCaseInstanceProvider {
            value: InstanceProviderType::InstancePerConstructorArgs,
        })
        .annotate(TestCaseOrdering::alphabetic_order())
        .constructor(Constructor::with_params(
            &[("java.lang.String", None), ("int", None)],
            |args| {
                let p = params_args(args);
                log(format!("constructor {}", p.value));
                p
            },
        ))
        .method(
            Method::new("parameters")
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory {
                    argument_formatting: "%2$s:%1$s".into(),
                    shuffle: false,
                })
                .static_value(|_| {
                    let row =
                        |s: &str, i: i32| Value::Array(vec![Value::Str(s.into()), Value::Int(i)]);
                    Value::List(vec![row("x", 10), row("y", 20)])
                }),
        )
        .method(
            Method::new("testZ")
                .annotate(Test::new())
                .body(|p: &mut ParamsInst| log(format!("testZ {}", p.value))),
        )
        .method(
            Method::new("testA")
                .annotate(Test::new())
                .body(|p: &mut ParamsInst| log(format!("testA {}", p.value))),
        )
        .method(
            Method::new("testM")
                .annotate(Test::new())
                .body(|p: &mut ParamsInst| log(format!("testM {}", p.value))),
        )
        .build()
});

struct AutoListener;

impl RunListener for AutoListener {
    fn test_run_started(&self, d: &Arc<Description>) {
        log(format!("auto runStarted {d}"));
    }
    fn test_run_finished(&self, r: &JUnitResult) {
        log(format!("auto runFinished run={}", r.run_count()));
    }
    fn test_started(&self, d: &Arc<Description>) {
        log(format!("auto started {d}"));
    }
}

static JUNIT3_STYLE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("JUnit3Style")
        .annotate(Listeners {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.RunnerOracle$AutoListener",
                || Arc::new(AutoListener) as Arc<dyn RunListener>,
            )],
        })
        .annotate(TestMethodProviders {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.JUnit3MethodProvider",
                || Arc::new(JUnit3MethodProvider) as Arc<dyn randomizedtesting::TestMethodProvider>,
            )],
        })
        .constructor(unit())
        .method(Method::new("test1").body_any(|| log("test1")))
        .method(Method::new("test2").body_any(|| log("test2")))
        .method(Method::new("helper").body_any(|| log("never")))
        .build()
});

struct JdkSupplier;

impl RandomSupplier for JdkSupplier {
    fn get(&self, seed: i64) -> Arc<dyn Random> {
        Arc::new(JdkRandom::new(seed))
    }
}

static DECORATED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Decorated")
        .annotate(SeedDecorators {
            value: vec![SeedDecoratorClass::mix_with_suite_name()],
        })
        .annotate(TestContextRandomSupplier {
            value: ClassRef::new(
                "com.carrotsearch.randomizedtesting.RunnerOracle$JdkSupplier",
                || Arc::new(JdkSupplier) as Arc<dyn RandomSupplier>,
            ),
        })
        .constructor(unit())
        .method(test("testOne", || log(format!("testOne {}", draw()))))
        .method(test("testTwo", || log(format!("testTwo {}", draw()))))
        .build()
});

static REPRODUCE_PRINTER_SUITE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ReproducePrinter$Suite")
        .annotate(Listeners {
            value: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.listeners.ReproduceInfoPrinter",
                || {
                    Arc::new(randomizedtesting::listeners::ReproduceInfoPrinter)
                        as Arc<dyn RunListener>
                },
            )],
        })
        .constructor(unit())
        .method(test("testFails", || runtime("reproduce me")))
        .build()
});

// -----------------------------------------------------------------------------
// Validation errors.
// -----------------------------------------------------------------------------

static NON_PUBLIC_TEST: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("NonPublicTest")
        .constructor(unit())
        .method(
            Method::new("testPackage")
                .modifiers(0)
                .annotate(Test::new())
                .body_any(|| {}),
        )
        .build()
});

static STATIC_TEST: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StaticTest")
        .constructor(unit())
        .method(
            Method::new("testStatic")
                .annotate(Test::new())
                .static_body(|| {}),
        )
        .build()
});

static ARGS_TEST: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ArgsTest")
        .constructor(unit())
        .method(
            Method::new("testArgs")
                .params(&["java.lang.String", "int"])
                .annotate(Test::new())
                .body_any(|| {}),
        )
        .build()
});

static INSTANCE_BEFORE_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("InstanceBeforeClass")
        .constructor(unit())
        .method(hook("beforeClass", BeforeClass, || {}))
        .method(test("testOne", || {}))
        .build()
});

static NO_FACTORY: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("NoFactory")
        .constructor(Constructor::with_params(
            &[("java.lang.String", None)],
            |_| (),
        ))
        .method(test("testOne", || {}))
        .build()
});

static BAD_METHOD_SEED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("BadMethodSeed")
        .constructor(unit())
        .method(test("testOne", || {}).annotate(Seed::new("1:2")))
        .build()
});

static SIMPLE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Simple")
        .constructor(unit())
        .method(test("testOne", || log(format!("testOne {}", draw()))))
        .method(test("testTwo", || log(format!("testTwo {}", draw()))))
        .build()
});

// -----------------------------------------------------------------------------
// Thread leaks, timeouts, uncaught exceptions.
// -----------------------------------------------------------------------------

fn sleeper(name: &'static str, millis: i64) -> Thread {
    Thread::new(name, move || {
        if Thread::try_sleep(millis).is_err() {
            log(format!("thread {name} interrupted"));
        }
    })
}

static LEAK_TEST: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakTest")
        .constructor(unit())
        .method(test("testLeak", || sleeper("leaked-test", 10000).start()))
        .method(test("testAfterLeak", || log("testAfterLeak")))
        .build()
});

/// `LeakRandomizedSleep`: a thread leaked while it sleeps with
/// `RandomizedTest.sleep`, which rethrows the leak control's interrupt.
static LEAK_RANDOMIZED_SLEEP: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakRandomizedSleep")
        .constructor(unit())
        .method(test("testLeak", || {
            Thread::new("leaked-randomized-sleep", || {
                let r = std::panic::catch_unwind(|| loop {
                    RandomizedTest::sleep(1000)
                });
                if let Err(p) = r {
                    log(format!(
                        "thread leaked-randomized-sleep ended by {}",
                        Throwable::from_panic(p)
                    ));
                }
            })
            .start()
        }))
        .build()
});

/// `@Ignore("whole class") IgnoredClass`: JUnit's `IgnoredClassRunner`.
static IGNORED_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("IgnoredClass")
        .annotate(Ignore::because("whole class"))
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            log("beforeClass")
        }))
        .method(test("testOne", || log("testOne")))
        .build()
});

static LEAK_SUITE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakSuite")
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            sleeper("leaked-suite", 10000).start()
        }))
        .method(test("testOne", || log("testOne")))
        .build()
});

static JOINED: Mutex<Option<Thread>> = Mutex::new(None);

fn join_saved() {
    if let Some(t) = JOINED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = t.join();
    }
}

fn start_saved(t: Thread) {
    t.start();
    *JOINED.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
}

fn after_class_join() -> Method {
    Method::new("afterClass")
        .throws(&["java.lang.Exception"])
        .annotate(AfterClass)
        .static_body(join_saved)
}

static LEAK_NONE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakNone")
        .annotate(ThreadLeakScope { value: Scope::None })
        .constructor(unit())
        .method(test("testOne", || {
            start_saved(sleeper("leaked-none", 300));
            log("testOne")
        }))
        .method(after_class_join())
        .build()
});

static LEAK_LINGER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakLinger")
        .annotate(ThreadLeakLingering { linger: 3000 })
        .constructor(unit())
        .method(test("testOne", || {
            sleeper("lingering", 300).start();
            log("testOne")
        }))
        .build()
});

static LEAK_WARN_ONLY: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakWarnOnly")
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn],
        })
        .constructor(unit())
        .method(test("testOne", || start_saved(sleeper("warned", 500))))
        .method(after_class_join())
        .build()
});

fn sleep_in_test(name: &'static str, millis: i64) {
    if Thread::try_sleep(millis).is_err() {
        log(format!("{name} interrupted"));
    }
}

static TEST_TIMEOUT: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("TestTimeout")
        .constructor(unit())
        .method(
            test("testSlow", || sleep_in_test("testSlow", 10000)).annotate(Timeout { millis: 300 }),
        )
        .method(
            Method::new("testJUnitTimeout")
                .annotate(Test::timeout(200))
                .body_any(|| sleep_in_test("testJUnitTimeout", 10000)),
        )
        .method(test("testFast", || log("testFast")).annotate(Timeout { millis: 5000 }))
        .build()
});

static SUITE_TIMEOUT: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("SuiteTimeout")
        .annotate(TimeoutSuite { millis: 1000 })
        .constructor(unit())
        .method(test("testA", || log("testA")))
        .method(test("testB", || log("testB")))
        .method(test("testHangs", || sleep_in_test("testHangs", 20000)))
        .build()
});

static UNCAUGHT: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Uncaught")
        .constructor(unit())
        .method(
            Method::new("testUncaught")
                .throws(&["java.lang.Exception"])
                .annotate(Test::new())
                .body_any(|| {
                    let t = Thread::new("uncaught-thread", || runtime("boom in thread"));
                    t.start();
                    let _ = t.join();
                    log("testUncaught joined");
                }),
        )
        .method(test("testClean", || log("testClean")))
        .build()
});

static UNCAUGHT_SUITE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("UncaughtSuite")
        .constructor(unit())
        .method(
            Method::new("beforeClass")
                .throws(&["java.lang.Exception"])
                .annotate(BeforeClass)
                .static_body(|| {
                    let t = Thread::new("uncaught-suite", || {
                        Throwable::new(ILLEGAL_STATE_EXCEPTION, "boom in suite thread").throw()
                    });
                    t.start();
                    let _ = t.join();
                }),
        )
        .method(test("testOne", || log("testOne")))
        .build()
});

struct RejectAll;

impl ThreadFilter for RejectAll {
    fn reject(&self, t: &Thread) -> bool {
        t.name().starts_with("filtered-")
    }
}

static LEAK_FILTERED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("LeakFiltered")
        .annotate(ThreadLeakFilters {
            default_filters: true,
            filters: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.RunnerOracle$RejectAll",
                || Arc::new(RejectAll) as Arc<dyn ThreadFilter>,
            )],
        })
        .constructor(unit())
        .method(test("testOne", || {
            start_saved(sleeper("filtered-1", 300));
            log("testOne")
        }))
        .method(after_class_join())
        .build()
});

static RELEASE_ZOMBIE: AtomicBool = AtomicBool::new(false);

static ZOMBIES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Zombies")
        .constructor(unit())
        .method(test("testA", || {
            let t = Thread::new("zombie", || {
                while !RELEASE_ZOMBIE.load(Ordering::Acquire) {
                    let _ = Thread::try_sleep(50);
                }
            });
            t.set_daemon(true);
            t.start();
            log("testA");
        }))
        .method(test("testB", || log("testB")))
        .method(test("testC", || log("testC")))
        .build()
});

static AFTER_ZOMBIES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("AfterZombies")
        .constructor(unit())
        .method(test("testOne", || log("never")))
        .build()
});

// -----------------------------------------------------------------------------
// Rules.
// -----------------------------------------------------------------------------

static RESTORE_RULE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("RestoreRule")
        .constructor(unit())
        .field(rule_field("restore", || {
            Value::TestRule(Arc::new(SystemPropertiesRestoreRule::new()))
        }))
        .field(rule_field("restoreSet", || {
            Value::TestRule(Arc::new(SystemPropertiesRestoreRule::with_property(
                "oracle.set",
                "during",
            )))
        }))
        .method(test("testSets", || {
            log(format!(
                "testSets oracle.set={}",
                system::get_property("oracle.set").unwrap_or_else(|| "null".into())
            ));
            system::set_property("oracle.prop", "changed");
        }))
        .method(test("testSees", || {
            log(format!(
                "testSees oracle.prop={} oracle.set={}",
                system::get_property("oracle.prop").unwrap_or_else(|| "null".into()),
                system::get_property("oracle.set").unwrap_or_else(|| "null".into())
            ))
        }))
        .build()
});

static INVARIANT_RULE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("InvariantRule")
        .constructor(unit())
        .field(rule_field("invariant", || {
            Value::TestRule(Arc::new(SystemPropertiesInvariantRule::ignoring(&[
                "oracle.ignored",
            ])))
        }))
        .method(test("testAdds", || {
            system::set_property("oracle.added", "x");
        }))
        .method(test("testChanges", || {
            system::set_property("oracle.existing", "changed");
        }))
        .method(test("testRemoves", || {
            system::clear_property("oracle.existing");
        }))
        .method(test("testIgnored", || {
            system::set_property("oracle.ignored", "y");
        }))
        .method(test("testClean", || log("testClean")))
        .build()
});

static SHADOW_SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ShadowSuper")
        .constructor(unit())
        .field(class_rule_field("noShadow", || {
            Value::TestRule(Arc::new(NoClassHooksShadowingRule::new()))
        }))
        .field(rule_field("noOverride", || {
            Value::TestRule(Arc::new(NoInstanceHooksOverridesRule::new()))
        }))
        .method(static_hook("setup", BeforeClass, || log("super setup")))
        .method(hook("init", Before, || log("super init")))
        .method(test("testOne", || log("testOne")))
        .build()
});

static SHADOW_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("ShadowSub")
        .extends(&SHADOW_SUPER)
        .constructor(unit())
        .method(static_hook("setup", BeforeClass, || log("sub setup")))
        .build()
});

static OVERRIDE_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("OverrideSub")
        .extends(&SHADOW_SUPER)
        .constructor(unit())
        .method(hook("init", Before, || log("sub init")))
        .build()
});

// `Method.invoke` dispatches virtually: the runner invokes the topmost
// annotated hook, which runs the receiver's most-derived override (an
// override without the annotation included); a private method is not
// overridden.
fn virtual_super_set_up() {
    log("super setUp");
}

fn virtual_super_tear_down() {
    log("super tearDown");
}

static VIRTUAL_SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("VirtualSuper")
        .constructor(unit())
        .method(hook("setUp", Before, virtual_super_set_up))
        .method(hook("packageHook", Before, || log("super packageHook")).modifiers(0))
        .method(
            hook("privateHook", Before, || log("super privateHook")).modifiers(modifier::PRIVATE),
        )
        .method(hook("tearDown", After, virtual_super_tear_down))
        .method(test("testOne", || log("testOne")))
        .build()
});

static VIRTUAL_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("VirtualSub")
        .extends(&VIRTUAL_SUPER)
        .constructor(unit())
        .method(Method::new("setUp").body_any(|| {
            log("sub setUp");
            virtual_super_set_up();
        }))
        .method(
            Method::new("packageHook")
                .modifiers(0)
                .body_any(|| log("sub packageHook")),
        )
        .method(
            Method::new("privateHook")
                .modifiers(modifier::PRIVATE)
                .body_any(|| log("sub privateHook")),
        )
        .method(Method::new("tearDown").body_any(|| {
            virtual_super_tear_down();
            log("sub tearDown");
        }))
        .build()
});

static VIRTUAL_SUB_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("VirtualSubSub")
        .extends(&VIRTUAL_SUB)
        .constructor(unit())
        .method(Method::new("setUp").body_any(|| log("subsub setUp")))
        .build()
});

static SF_BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StaticFields$Base")
        .constructor(unit())
        .field(
            Field::new_static("rule", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(StaticFieldsInvariantRule::with_threshold(
                    1024 * 1024,
                    true,
                )))
            })
            .modifiers(modifier::PUBLIC | modifier::FINAL)
            .annotate(ClassRule),
        )
        .method(test("testOne", || log("testOne")))
        .build()
});

static SF0: Mutex<Value> = Mutex::new(Value::Null);
static SF1: Mutex<Value> = Mutex::new(Value::Null);
static SF2: Mutex<Value> = Mutex::new(Value::Null);
static SF3: Mutex<Value> = Mutex::new(Value::Null);
static SF4: Mutex<Value> = Mutex::new(Value::Null);
static SF5: LazyLock<Value> = LazyLock::new(|| Value::primitive_array("long", 1024 * 1024));
static SMALL: Mutex<Value> = Mutex::new(Value::Null);
static TEXT: Mutex<Value> = Mutex::new(Value::Null);
static OK: Mutex<Value> = Mutex::new(Value::Null);

fn get(m: &'static Mutex<Value>) -> impl Fn() -> Value + Send + Sync + 'static {
    move || m.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(m: &'static Mutex<Value>, v: Value) {
    *m.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

fn static_field(name: &str, ty: &str, m: &'static Mutex<Value>) -> Field {
    Field::new_static(name, ty, get(m)).modifiers(0)
}

static SF_EXCEEDING: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StaticFields$Exceeding")
        .extends(&SF_BASE)
        .constructor(unit())
        .field(static_field("field0", "byte[]", &SF0))
        .field(static_field("field1", "long[]", &SF1))
        .field(static_field("field2", "int[]", &SF2))
        .field(static_field("field3", "java.lang.Object", &SF3))
        .field(static_field("field4", "java.lang.Long", &SF4))
        .field(Field::new_static("field5", "long[]", || SF5.clone()).modifiers(modifier::FINAL))
        .method(static_hook("setup", BeforeClass, || {
            set(&SF0, Value::primitive_array("byte", 600 * 1024));
            set(&SF1, Value::primitive_array("long", 64 * 1024));
            set(&SF2, Value::primitive_array("int", 100));
            set(
                &SF3,
                Value::Array(vec![
                    Value::primitive_array("byte", 16),
                    Value::Null,
                    Value::Int(7),
                ]),
            );
            set(&SF4, Value::Long(42));
        }))
        .build()
});

static SF_UNDER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StaticFields$Under")
        .extends(&SF_BASE)
        .constructor(unit())
        .field(static_field("small", "byte[]", &SMALL))
        .method(static_hook("setup", BeforeClass, || {
            set(&SMALL, Value::primitive_array("byte", 1000))
        }))
        .build()
});

static SF_UNMEASURABLE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("StaticFields$Unmeasurable")
        .extends(&SF_BASE)
        .constructor(unit())
        .field(static_field("text", "java.lang.String", &TEXT))
        .field(static_field("ok", "int[]", &OK))
        .method(static_hook("setup", BeforeClass, || {
            set(&TEXT, Value::Str("hello".into()));
            set(&OK, Value::primitive_array("int", 3));
        }))
        .build()
});

struct AdapterRule;

impl TestRuleAdapter for AdapterRule {
    fn before(&self) {
        log("adapter before");
    }
    fn after_if_successful(&self) {
        log("adapter afterIfSuccessful");
    }
    fn after_always(&self, errors: &mut Vec<Throwable>) {
        log(format!("adapter afterAlways errors={}", errors.len()));
        runtime("afterAlways adds")
    }
}

static ADAPTERS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("Adapters")
        .constructor(unit())
        .field(rule_field("adapter", || {
            Value::TestRule(Arc::new(TestRuleAdapterRule(Arc::new(AdapterRule))))
        }))
        .method(test("testPass", || log("testPass")))
        .method(test("testFail", || runtime("test fails")))
        .build()
});

static ASSERTIONS_CLASS: OnceLock<Arc<Class>> = OnceLock::new();

fn assertions() -> &'static Arc<Class> {
    ASSERTIONS_CLASS.get_or_init(|| {
        cls("Assertions")
            .constructor(unit())
            .field(
                Field::new_static("rule", "org.junit.rules.TestRule", || {
                    let c = Arc::clone(ASSERTIONS_CLASS.get().expect("Assertions class"));
                    Value::TestRule(Arc::new(TestRuleAdapterRule(Arc::new(
                        RequireAssertionsRule::new(c),
                    ))))
                })
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(ClassRule),
            )
            .method(test("testOne", || log("testOne")))
            .build()
    })
}

// -----------------------------------------------------------------------------
// The scenarios, in the oracle's order.
// -----------------------------------------------------------------------------

fn run_scenarios() -> String {
    let mut w = String::new();
    let w = &mut w;
    let s = "tests.seed";
    scenario(w, "hooks-deadbeef", &HOOK_SUB, &[(s, "DEADBEEF")]);
    scenario(w, "hooks-cafebabe", &HOOK_SUB, &[(s, "CAFEBABE")]);
    scenario(w, "hooks-seed-chain", &HOOK_SUB, &[(s, "DEADBEEF:1234")]);
    scenario(w, "seeds-annotations", &SEEDS1, &[]);
    scenario(w, "seeds-annotations-overridden", &SEEDS1, &[(s, "1")]);
    scenario(w, "seed-chain-on-class", &SEED_CHAIN_ON_CLASS, &[]);
    scenario(w, "repeat", &REPEATS, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "repeat-iters-property",
        &REPEATS,
        &[(s, "DEADBEEF"), ("tests.iters", "2")],
    );
    scenario(w, "repeat-on-class", &REPEATS_ON_CLASS, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "repeat-append-seed",
        &SIMPLE,
        &[(s, "DEADBEEF"), ("tests.appendseed", "true")],
    );
    scenario(w, "failures", &FAILURES, &[(s, "DEADBEEF")]);
    scenario(w, "hook-failures", &HOOK_FAILURES, &[(s, "DEADBEEF")]);
    scenario(w, "class-assumption", &CLASS_ASSUMPTION, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "before-class-failure",
        &BEFORE_CLASS_FAILURE,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "groups-default", &GROUPS, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "groups-nightly",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.nightly", "true")],
    );
    scenario(
        w,
        "groups-slow-disabled-fast",
        &GROUPS,
        &[
            (s, "DEADBEEF"),
            ("tests.slow", "true"),
            ("oracle.fast", "false"),
        ],
    );
    scenario(
        w,
        "groups-malformed",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.slow", "maybe")],
    );
    scenario(
        w,
        "filter-slow-and-not-nightly",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.filter", "@slow and not @nightly")],
    );
    scenario(
        w,
        "filter-default-or-nightly",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.filter", "default or @NIGHTLY")],
    );
    scenario(
        w,
        "filter-not-default",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.filter", "not (default)")],
    );
    scenario(
        w,
        "filter-syntax-error",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.filter", "@slow and")],
    );
    scenario(
        w,
        "filter-bad-token",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.filter", "slow")],
    );
    scenario(w, "nightly-suite", &NIGHTLY_SUITE, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "method-filter",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.method", "test*l*")],
    );
    scenario(
        w,
        "method-filter-none",
        &GROUPS,
        &[(s, "DEADBEEF"), ("tests.method", "nothing")],
    );
    scenario(
        w,
        "method-filter-repeat",
        &REPEATS,
        &[(s, "DEADBEEF"), ("tests.method", "testRepeat")],
    );
    scenario(
        w,
        "class-filter-none",
        &SIMPLE,
        &[(s, "DEADBEEF"), ("tests.class", "*Other*")],
    );
    scenario(
        w,
        "class-filter-match",
        &SIMPLE,
        &[(s, "DEADBEEF"), ("tests.class", "*RunnerOracle$Sim?le")],
    );
    scenario(w, "params", &PARAMS, &[(s, "DEADBEEF")]);
    scenario(w, "params-formatted", &PARAMS_FORMATTED, &[(s, "DEADBEEF")]);
    scenario(w, "junit3-listeners", &JUNIT3_STYLE, &[(s, "DEADBEEF")]);
    scenario(w, "decorated", &DECORATED, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "invalid-non-public",
        &NON_PUBLIC_TEST,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "invalid-static", &STATIC_TEST, &[(s, "DEADBEEF")]);
    scenario(w, "invalid-args", &ARGS_TEST, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "invalid-instance-before-class",
        &INSTANCE_BEFORE_CLASS,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "invalid-no-factory", &NO_FACTORY, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "invalid-method-seed",
        &BAD_METHOD_SEED,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "invalid-tests-seed", &SIMPLE, &[(s, "XYZ")]);
    scenario(w, "invalid-tests-seed-chain", &SIMPLE, &[(s, "1:2:3")]);
    scenario(
        w,
        "invalid-tests-iters",
        &SIMPLE,
        &[(s, "DEADBEEF"), ("tests.iters", "0")],
    );
    {
        let buf = Arc::new(Mutex::new(String::new()));
        let b2 = Arc::clone(&buf);
        let saved = system::set_err(Some(Arc::new(move |t: &str| {
            b2.lock().unwrap_or_else(|e| e.into_inner()).push_str(t)
        })));
        scenario(
            w,
            "reproduce-info-printer",
            &REPRODUCE_PRINTER_SUITE,
            &[(s, "DEADBEEF"), ("tests.iters", "1")],
        );
        system::set_err(saved);
        let text = buf.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for line in text.split('\n') {
            if ["FAILURE", "Message", "Reproduce", "Throwable", "java."]
                .iter()
                .any(|p| line.starts_with(p))
            {
                w.push_str(&format!("stderr {line}\n"));
            }
        }
    }
    scenario(
        w,
        "leak-test",
        &LEAK_TEST,
        &[(s, "DEADBEEF"), ("tests.killwait", "200")],
    );
    scenario(
        w,
        "leak-suite",
        &LEAK_SUITE,
        &[(s, "DEADBEEF"), ("tests.killwait", "200")],
    );
    scenario(w, "leak-none", &LEAK_NONE, &[(s, "DEADBEEF")]);
    scenario(w, "leak-linger", &LEAK_LINGER, &[(s, "DEADBEEF")]);
    scenario(w, "leak-warn-only", &LEAK_WARN_ONLY, &[(s, "DEADBEEF")]);
    scenario(w, "leak-filtered", &LEAK_FILTERED, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "leak-randomized-sleep",
        &LEAK_RANDOMIZED_SLEEP,
        &[(s, "DEADBEEF"), ("tests.killwait", "200")],
    );
    scenario(w, "class-ignore", &IGNORED_CLASS, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "timeout-test",
        &TEST_TIMEOUT,
        &[(s, "DEADBEEF"), ("tests.killwait", "200")],
    );
    scenario(
        w,
        "timeout-test-global-first",
        &TEST_TIMEOUT,
        &[
            (s, "DEADBEEF"),
            ("tests.timeout", "100!"),
            ("tests.killwait", "200"),
        ],
    );
    scenario(
        w,
        "timeout-suite",
        &SUITE_TIMEOUT,
        &[(s, "DEADBEEF"), ("tests.killwait", "200")],
    );
    scenario(w, "uncaught", &UNCAUGHT, &[(s, "DEADBEEF")]);
    scenario(w, "uncaught-suite", &UNCAUGHT_SUITE, &[(s, "DEADBEEF")]);
    system::set_property("oracle.existing", "before");
    scenario(w, "rule-restore", &RESTORE_RULE, &[(s, "DEADBEEF")]);
    let p = |k: &str| system::get_property(k).unwrap_or_else(|| "null".into());
    w.push_str(&format!(
        "after oracle.prop={} oracle.set={}\n",
        p("oracle.prop"),
        p("oracle.set")
    ));
    scenario(w, "rule-invariant", &INVARIANT_RULE, &[(s, "DEADBEEF")]);
    w.push_str(&format!(
        "after oracle.added={} oracle.existing={} oracle.ignored={}\n",
        p("oracle.added"),
        p("oracle.existing"),
        p("oracle.ignored")
    ));
    system::clear_property("oracle.existing");
    system::clear_property("oracle.ignored");
    scenario(w, "rule-no-shadow-pass", &SHADOW_SUPER, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "rule-no-class-hooks-shadowing",
        &SHADOW_SUB,
        &[(s, "DEADBEEF")],
    );
    scenario(
        w,
        "rule-no-instance-hooks-overrides",
        &OVERRIDE_SUB,
        &[(s, "DEADBEEF")],
    );
    scenario(
        w,
        "virtual-dispatch-super",
        &VIRTUAL_SUPER,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "virtual-dispatch-sub", &VIRTUAL_SUB, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "virtual-dispatch-subsub",
        &VIRTUAL_SUB_SUB,
        &[(s, "DEADBEEF")],
    );
    scenario(
        w,
        "rule-static-fields-exceeding",
        &SF_EXCEEDING,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "rule-static-fields-under", &SF_UNDER, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "rule-static-fields-unmeasurable",
        &SF_UNMEASURABLE,
        &[(s, "DEADBEEF")],
    );
    scenario(w, "rule-adapter", &ADAPTERS, &[(s, "DEADBEEF")]);
    scenario(
        w,
        "require-assertions-match",
        assertions(),
        &[(s, "DEADBEEF")],
    );
    scenario(
        w,
        "zombies",
        &ZOMBIES,
        &[
            (s, "DEADBEEF"),
            ("tests.killattempts", "2"),
            ("tests.killwait", "100"),
        ],
    );
    scenario(w, "after-zombies", &AFTER_ZOMBIES, &[(s, "DEADBEEF")]);
    RELEASE_ZOMBIE.store(true, Ordering::Release);
    w.clone()
}

// -----------------------------------------------------------------------------
// Masking and comparison.
// -----------------------------------------------------------------------------

/// Replaces the digits after each `prefix` with `N`.
fn mask_digits(s: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(prefix) {
        out.push_str(&rest[..i + prefix.len()]);
        rest = &rest[i + prefix.len()..];
        let n = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if n > 0 {
            out.push('N');
            rest = &rest[n..];
        }
    }
    out.push_str(rest);
    out
}

/// Replaces the hexadecimal digits after each `prefix` (an identity hash)
/// with `H`.
fn mask_hex_after(s: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(prefix) {
        out.push_str(&rest[..i + prefix.len()]);
        rest = &rest[i + prefix.len()..];
        let n = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        out.push('H');
        rest = &rest[n..];
    }
    out.push_str(rest);
    out
}

/// Replaces each `state=STATE` with `state=S`.
fn mask_state(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("state=") {
        out.push_str(&rest[..i + 6]);
        rest = &rest[i + 6..];
        let n = rest
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || *c == '_')
            .count();
        out.push('S');
        rest = &rest[n..];
    }
    out.push_str(rest);
    out
}

/// Collapses each run of escaped stack frames (`\n        at …`) into one
/// `\n        at <frames>`.
fn mask_frames(s: &str) -> String {
    const FRAME: &str = "\\n        at ";
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(FRAME) {
        out.push_str(&rest[..i]);
        out.push_str(FRAME);
        out.push_str("<frames>");
        rest = &rest[i..];
        loop {
            if !rest.starts_with(FRAME) {
                break;
            }
            let body = &rest[FRAME.len()..];
            let end = [body.find("\\n"), body.find(" [seed "), body.find(" <- ")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(body.len());
            rest = &body[end..];
        }
    }
    out.push_str(rest);
    out
}

fn mask(s: &str) -> String {
    s.lines()
        .map(|l| {
            let l = mask_frames(l);
            let l = mask_digits(&l, "id=");
            let l = mask_digits(&l, "Thread[#");
            let l = mask_hex_after(&l, "unnamed module @");
            mask_state(&l)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn blocks(s: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for line in s.lines() {
        if let Some(name) = line.strip_prefix("=== ") {
            v.push((name.to_string(), String::new()));
        } else if let Some((_, b)) = v.last_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    v
}

fn compare(label: &str, expected: &str, actual: &str) -> usize {
    let e = blocks(&mask(expected));
    let a = blocks(&mask(actual));
    let mut failures = 0;
    for (name, eb) in &e {
        match a.iter().find(|(n, _)| n == name) {
            None => {
                println!("[{label}] {name}: MISSING in the Rust trace");
                failures += 1;
            }
            Some((_, ab)) if ab == eb => println!("[{label}] {name}: ok"),
            Some((_, ab)) => {
                failures += 1;
                println!("[{label}] {name}: DIFFERS");
                let el: Vec<&str> = eb.lines().collect();
                let al: Vec<&str> = ab.lines().collect();
                for i in 0..el.len().max(al.len()) {
                    let x = el.get(i).copied().unwrap_or("<none>");
                    let y = al.get(i).copied().unwrap_or("<none>");
                    if x != y {
                        println!("  line {i}:\n    java: {x}\n    rust: {y}");
                    }
                }
            }
        }
    }
    if a.len() != e.len() {
        println!("[{label}] block count: java {} rust {}", e.len(), a.len());
        failures += 1;
    }
    failures
}

fn main() {
    // The JVM's platform property the reproduce line lists.
    system::set_property("file.encoding", "UTF-8");
    let _h = logging::add_handler(Arc::new(|_name: &str, level: Level, message: &str, _t| {
        let first = message.split('\n').next().unwrap_or("");
        log(format!("log {level} {first}"));
    }));
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "asserts-false") {
        system::set_property("tests.asserts", "false");
        let mut w = String::new();
        scenario(
            &mut w,
            "require-assertions-mismatch",
            assertions(),
            &[("tests.seed", "DEADBEEF")],
        );
        print!("{w}");
        return;
    }
    let actual = run_scenarios();
    if let Ok(dir) = std::env::var("RUNNER_ORACLE_DUMP") {
        let _ = std::fs::write(format!("{dir}/runner_traces_rust.tsv"), &actual);
    }
    let mut failures = compare("runner", &fixture("runner_traces.tsv"), &actual);
    // RequireAssertionsRule reads `tests.asserts` once per process, as
    // Java reads it once per JVM: the mismatch case runs in a child.
    let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("asserts-false")
        .output()
        .expect("run the asserts-false child");
    let child_out = String::from_utf8_lossy(&child.stdout).to_string();
    failures += compare("asserts", &fixture("runner_traces_asserts.tsv"), &child_out);
    if failures > 0 {
        println!("runner oracle: {failures} difference(s)");
        std::process::exit(1);
    }
    println!("runner oracle: all scenarios equal the Java trace");
}
