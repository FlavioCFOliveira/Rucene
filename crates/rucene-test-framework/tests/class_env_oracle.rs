//! Portability test of tasks #348 and #226 against `ClassEnvOracle`
//! (`crates/randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/tests/util/`):
//!
//! * `class_env.tsv`: for each `tests.*` configuration (one child process
//!   each, as the `LuceneTestCase` constants are computed once per process)
//!   and each seed of `OracleSupport.seeds()`, the class environment
//!   `TestRuleSetupAndRestoreClassEnv` sets up, the order of a suite's own
//!   rules and hooks inside `LuceneTestCase`'s chains, the draws of each
//!   scope, the reproduce line and test params of
//!   `RunListenerPrintReproduceInfo`, `@SuppressCodecs` and what `after()`
//!   restores;
//! * `string_helper_ids.tsv`: `StringHelper.randomId()` for fixed
//!   `tests.seed` values.

mod oracle_support;

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use oracle_support::rows;
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{
    After, AfterClass, Before, BeforeClass, ClassRule, Description, JUnitCore, Request, Rule,
    Statement, Test, TestRule,
};
use rucene::search::IndexSearcher;
use rucene_test_framework::util::lucene_test_case::{
    LiveIWCFlushMode, LuceneTestCase, SuppressCodecs, MAYBE_CACHE_POLICY,
};
use rucene_test_framework::util::lucene_test_case_rules::CLASS_ENV_RULE;

/// The child process's environment variable naming its configuration.
const CHILD: &str = "RUCENE_CLASS_ENV_ORACLE_CHILD";

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn event(e: String) {
    EVENTS.lock().unwrap().push(e);
}

/// `TestClassEnvProbe.env()`.
fn env() -> String {
    let codec = rucene::codecs::get_default().map_or("null".to_string(), |c| c.to_java_string());
    let sim = CLASS_ENV_RULE
        .env()
        .similarity
        .as_ref()
        .map_or("null".to_string(), |s| s.to_string());
    let locale = rucene_test_framework::jdk::locale::get_default()
        .map_or("null".to_string(), |l| l.to_language_tag());
    let tz = LuceneTestCase::get_default_time_zone().map_or("null", |t| t.get_id());
    let info = rucene::util::default_info_stream().java_class_name();
    let flush = match LuceneTestCase::live_iwc_flush_mode() {
        LiveIWCFlushMode::ByRam => "BY_RAM",
        LiveIWCFlushMode::ByDocs => "BY_DOCS",
        LiveIWCFlushMode::Either => "EITHER",
    };
    let cache = match IndexSearcher::get_default_query_cache() {
        None => "null".to_string(),
        Some(c) => {
            let d = format!("{c:?}");
            d.split([' ', '{', '(']).next().unwrap_or("").to_string()
        }
    };
    let policy = IndexSearcher::get_default_query_caching_policy()
        .is_some_and(|p| Arc::ptr_eq(&p, &MAYBE_CACHE_POLICY));
    let cores = system::get_property("lucene.cms.override_core_count")
        .unwrap_or_else(|| "null".to_string());
    let executor =
        LuceneTestCase::executor().map_or("null".to_string(), |e| e.threads().to_string());
    format!(
        "codec={codec}\tsim={sim}\tlocale={locale}\ttz={tz}\tinfo={info}\tflush={flush}\t\
         cache={cache}\tpolicy={policy}\tcores={cores}\texecutor={executor}"
    )
}

fn next() -> i64 {
    LuceneTestCase::random().next_long()
}

/// `TestClassEnvProbe.classProbe`.
struct ClassProbe;

impl TestRule for ClassProbe {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        Statement::new(move || {
            let codec = rucene::codecs::get_default().unwrap().to_java_string();
            event(format!(
                "classRule.before\tcodec={codec}\tclass={}",
                LuceneTestCase::get_test_class().simple_name()
            ));
            struct After;
            impl Drop for After {
                fn drop(&mut self) {
                    let codec = rucene::codecs::get_default().unwrap().to_java_string();
                    event(format!("classRule.after\tcodec={codec}"));
                }
            }
            let _after = After;
            base.evaluate();
        })
    }
}

/// `TestClassEnvProbe.testProbe`.
struct TestProbe;

impl TestRule for TestProbe {
    fn apply(&self, base: Statement, d: &Arc<Description>) -> Statement {
        let name = d.method_name().unwrap_or_default();
        Statement::new(move || {
            event(format!("rule.before\t{name}"));
            struct After(String);
            impl Drop for After {
                fn drop(&mut self) {
                    event(format!("rule.after\t{}", self.0));
                }
            }
            let _after = After(name.clone());
            base.evaluate();
        })
    }
}

fn probe_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestClassEnvProbe")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("classProbe", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(ClassProbe) as Arc<dyn TestRule>)
            })
            .annotate(ClassRule),
        )
        .field(
            Field::new_instance::<()>("testProbe", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(TestProbe) as Arc<dyn TestRule>)
            })
            .annotate(Rule),
        )
        .method(
            Method::new("probeBeforeClass")
                .annotate(BeforeClass)
                .static_body(|| event(format!("beforeClass\t{}\tnext={}", env(), next()))),
        )
        .method(
            Method::new("probeBefore")
                .annotate(Before)
                .body_any(|| event(format!("before\t{}", next()))),
        )
        .method(Method::new("testProbe").annotate(Test::new()).body_any(|| {
            let name = LuceneTestCase::current_test_rules()
                .and_then(|r| r.thread_and_test_name_rule.test_method_name())
                .unwrap_or_else(|| "null".to_string());
            event(format!("test\tname={name}\t{}\tnext={}", env(), next()));
        }))
        .method(
            Method::new("probeAfter")
                .annotate(After)
                .body_any(|| event(format!("after\t{}", next()))),
        )
        .method(
            Method::new("probeAfterClass")
                .annotate(AfterClass)
                .static_body(|| event(format!("afterClass\t{}", next()))),
        )
        .build()
}

fn failing_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestClassEnvProbe$TestClassEnvFailing")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("testFails")
                .annotate(Test::new())
                .body_any(|| randomizedtesting::junit::Assert::fail("boom")),
        )
        .build()
}

fn suppressed_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestClassEnvProbe$TestClassEnvSuppressed")
        .extends(&LuceneTestCase::class())
        .annotate(SuppressCodecs {
            value: ["Lucene104", "MockRandom", "Direct", "FST50", "SimpleText"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        })
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("testCodec").annotate(Test::new()).body_any(|| {
            let codec = rucene::codecs::get_default().unwrap().to_java_string();
            event(format!("suppressed\tcodec={codec}"));
        }))
        .build()
}

/// `Child.run`.
fn run(rows: &mut Vec<String>, err: &Mutex<String>, seed: &str, kind: &str, suite: &Arc<Class>) {
    EVENTS.lock().unwrap().clear();
    err.lock().unwrap().clear();
    let r = JUnitCore::new().run(&Request::a_class(suite));
    let p = format!("{seed}\t{kind}\t");
    rows.push(format!(
        "{p}result\trun={}\tfailures={}\tignored={}\tassumptions={}",
        r.run_count(),
        r.failure_count(),
        r.ignore_count(),
        r.assumption_failure_count()
    ));
    for f in r.failures() {
        rows.push(format!(
            "{p}failure\t{}\t{}",
            f.exception().class_name(),
            f.message().unwrap_or("null").replace('\n', "\\n")
        ));
    }
    for e in EVENTS.lock().unwrap().iter() {
        rows.push(format!("{p}event\t{e}"));
    }
    for line in err.lock().unwrap().split('\n') {
        if line.starts_with("NOTE: reproduce with:")
            || line.starts_with("NOTE: test params are:")
            || line.starts_with("NOTE: Class not allowed")
            || line.starts_with("Property '")
        {
            rows.push(format!("{p}stderr\t{line}"));
        }
    }
    rows.push(format!(
        "{p}restored\tcodec={}\tinfo={}",
        rucene::codecs::get_default().unwrap().name(),
        rucene::util::default_info_stream().java_class_name()
    ));
}

/// `SplittableRandom`'s gamma.
const GOLDEN_GAMMA: u64 = 0x9e3779b97f4a7c15;

/// `SplittableRandom.mix64`.
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// `OracleSupport.seeds()`: edge values, then ten draws of
/// `SplittableRandom(20261103)`.
fn seeds() -> Vec<i64> {
    let mut s = vec![
        0,
        1,
        -1,
        i64::MIN,
        i64::MAX,
        42,
        0x5DEECE66D,
        0xDEADBEEF,
        0x123456789ABCDEF,
        -0x123456789ABCDEF,
    ];
    // SplittableRandom(20261103).nextLong(), ten times.
    let mut seed: u64 = 20261103;
    for _ in 0..10 {
        seed = seed.wrapping_add(GOLDEN_GAMMA);
        s.push(mix64(seed) as i64);
    }
    s
}

/// `ClassEnvOracle.Child`: runs in the child process of one configuration
/// and prints its rows, prefixed with `ROW\t`.
#[test]
fn child_rows() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    // Captured before LuceneTestCase initializes, so that
    // TestRuleLimitSysouts wraps it.
    let err = Arc::new(Mutex::new(String::new()));
    {
        let err = Arc::clone(&err);
        system::set_err(Some(Arc::new(move |s: &str| {
            err.lock().unwrap().push_str(s)
        })));
    }
    let (probe, failing, suppressed) = (probe_class(), failing_class(), suppressed_class());
    // Java's first `TestClassEnvProbe.EVENTS.clear()` initializes the class,
    // and so `LuceneTestCase`, before the first run.
    probe.initialize().unwrap();
    let mut out = Vec::new();
    for (i, seed) in seeds().into_iter().enumerate() {
        let seed = format!("{:X}", seed as u64);
        system::set_property("tests.seed", &seed);
        run(&mut out, &err, &seed, "probe", &probe);
        if i < 4 {
            run(&mut out, &err, &seed, "failing", &failing);
        }
        run(&mut out, &err, &seed, "suppressed", &suppressed);
    }
    // A wider grid of the probe alone, so that every codec branch is drawn:
    // `SplittableRandom(348).nextLong()`, sixty times.
    let mut state: u64 = 348;
    for _ in 0..60 {
        state = state.wrapping_add(GOLDEN_GAMMA);
        let seed = format!("{:X}", mix64(state));
        system::set_property("tests.seed", &seed);
        run(&mut out, &err, &seed, "probe", &probe);
    }
    system::clear_property("tests.seed");
    for r in out {
        println!("ROW\t{r}");
    }
}

/// Java's `gradlew test --tests T -Dk=v...` as the port prints it:
/// `env k=v... cargo test --tests T` (`RunListenerPrintReproduceInfo`).
fn cargo_reproduce_line(java: &str) -> String {
    let rest = java
        .strip_prefix("NOTE: reproduce with: gradlew test --tests ")
        .expect("a Java reproduce line");
    let mut parts = rest.splitn(2, " -D");
    let target = parts.next().unwrap();
    let opts = parts
        .next()
        .map(|o| format!(" {}", o.replace(" -D", " ")))
        .unwrap_or_default();
    format!("NOTE: reproduce with: env{opts} cargo test --tests {target}")
}

#[test]
fn class_env_matches_lucene() {
    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows("class_env.tsv") {
        let mut line = row[1..].join("\t");
        if row[3] == "stderr" && row[4].starts_with("NOTE: reproduce with:") {
            line = format!(
                "{}\t{}",
                row[1..4].join("\t"),
                cargo_reproduce_line(&row[4])
            );
        }
        expected.entry(row[0].clone()).or_default().push(line);
    }
    assert_eq!(expected.len(), 15, "configurations");
    let exe = std::env::current_exe().unwrap();
    let mut mismatches = Vec::new();
    for (config, want) in &expected {
        let mut cmd = Command::new(&exe);
        cmd.args(["child_rows", "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1");
        if config != "-" {
            for prop in config.split(' ') {
                let (k, v) = prop.split_once('=').unwrap();
                cmd.env(k, v);
            }
        }
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let got: Vec<String> = stdout
            .lines()
            // libtest's `test child_rows ... ` may precede the first row.
            .filter_map(|l| l.find("ROW\t").map(|i| l[i + 4..].to_string()))
            .collect();
        assert!(
            output.status.success() && !got.is_empty(),
            "{config}: child failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let n = want.len().max(got.len());
        for i in 0..n {
            let (w, g) = (want.get(i), got.get(i));
            if w != g {
                mismatches.push(format!("{config} #{i}\n  java: {w:?}\n  rust: {g:?}"));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// `StringHelper.randomId()` under `-Dtests.seed=seed`: the first three ids
/// of `nextId` as the static initializer seeds it.
#[test]
fn string_helper_ids_match_lucene() {
    let mut n = 0;
    for row in rows("string_helper_ids.tsv") {
        let prop = if row[0] == "(empty)" {
            ""
        } else {
            row[0].as_str()
        };
        let outcome =
            std::panic::catch_unwind(|| rucene::util::string_helper::initial_next_id(Some(prop)));
        match outcome {
            Ok(first) => {
                let ids: Vec<String> = (0..3u128)
                    .map(|i| {
                        rucene::util::StringHelper::id_to_string(Some(
                            &first.wrapping_add(i).to_be_bytes(),
                        ))
                    })
                    .collect();
                assert_eq!(ids, row[1..].to_vec(), "tests.seed={prop}");
            }
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(row[1], "error", "tests.seed={prop}: {message}");
                assert_eq!(
                    message,
                    format!("{}: {}", row[2], row[3]),
                    "tests.seed={prop}"
                );
            }
        }
        n += 1;
    }
    assert_eq!(n, 8);
}
