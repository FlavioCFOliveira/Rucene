//! Task #338 oracle parity: `LuceneTestCase`'s static environment, random
//! helpers and deterministic statics against Lucene 10.5.0 on JDK 25
//! (`tests/fixtures/ltc_env.tsv`, `ltc_draws.tsv`, `ltc_statics.tsv`,
//! `ltc_locales.tsv`, written by `LuceneTestCaseOracle.java`).

mod oracle_support;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use oracle_support::{assert_same, labelled, rows};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{ClassRule, JUnitCore, Request, Test};
use rucene::util::BytesRef;
use rucene_test_framework::hamcrest::{equal_to, is};
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase as Ltc, StaticEnvironment, CLASS_NAME_RULE, MAYBE_CACHE_POLICY,
};
use rucene_test_framework::util::TestUtil;

/// The configurations of `LuceneTestCaseOracle.CONFIGS`, in its order.
const CONFIGS: &[&[&str]] = &[
    &[],
    &["tests.verbose=true"],
    &["tests.verbose=true", "tests.infostream=false"],
    &["tests.infostream=true"],
    &["tests.nightly=true"],
    &["tests.nightly=true", "tests.multiplier=5"],
    &["tests.multiplier=3"],
    &[
        "tests.codec=SimpleText",
        "tests.postingsformat=Direct",
        "tests.docvaluesformat=Asserting",
        "tests.directory=NIOFSDirectory",
        "tests.linedocsfile=x.txt",
    ],
    &[
        "tests.weekly=true",
        "tests.monster=yes",
        "tests.awaitsfix=on",
    ],
    &["tests.leavetmpdir=true"],
    &["tests.leaveTemporary=true"],
    &["tests.leavetemporary=true"],
    &["tests.asserts=false"],
    &["tests.verbose=bogus"],
    &["tests.multiplier=abc"],
    &["tests.nightly= TRUE "],
];

/// The `tests.*` properties the configurations touch, cleared around each.
const PROPERTIES: &[&str] = &[
    "tests.verbose",
    "tests.infostream",
    "tests.asserts",
    "tests.codec",
    "tests.postingsformat",
    "tests.docvaluesformat",
    "tests.directory",
    "tests.linedocsfile",
    "tests.nightly",
    "tests.weekly",
    "tests.monster",
    "tests.awaitsfix",
    "tests.multiplier",
    "tests.leaveTemporary",
    "tests.leavetemporary",
    "tests.leavetmpdir",
];

/// The process-wide property table is shared by the tests of this binary.
static PROPS: Mutex<()> = Mutex::new(());

#[test]
fn static_environment_matches_lucene() {
    let _guard = PROPS.lock().unwrap_or_else(|e| e.into_inner());
    let mut actual = BTreeMap::new();
    let mut put = |config: &str, k: &str, v: String| {
        actual.insert(format!("{config}\t{k}"), v);
    };
    put("-", "SYSPROP_NIGHTLY", Ltc::SYSPROP_NIGHTLY.into());
    put("-", "SYSPROP_WEEKLY", Ltc::SYSPROP_WEEKLY.into());
    put("-", "SYSPROP_MONSTER", Ltc::SYSPROP_MONSTER.into());
    put("-", "SYSPROP_AWAITSFIX", Ltc::SYSPROP_AWAITSFIX.into());
    put("-", "SYSPROP_MAXFAILURES", Ltc::SYSPROP_MAXFAILURES.into());
    put("-", "SYSPROP_FAILFAST", Ltc::SYSPROP_FAILFAST.into());
    put(
        "-",
        "DEFAULT_LINE_DOCS_FILE",
        Ltc::DEFAULT_LINE_DOCS_FILE.into(),
    );
    put(
        "-",
        "JENKINS_LARGE_LINE_DOCS_FILE",
        Ltc::JENKINS_LARGE_LINE_DOCS_FILE.into(),
    );
    for config in CONFIGS {
        for p in PROPERTIES {
            system::clear_property(p);
        }
        for kv in *config {
            let (k, v) = kv.split_once('=').unwrap();
            system::set_property(k, v);
        }
        let label = if config.is_empty() {
            "{}".to_string()
        } else {
            config.join(";")
        };
        match catch_unwind(AssertUnwindSafe(StaticEnvironment::from_system_properties)) {
            Ok(env) => {
                put(&label, "VERBOSE", env.verbose.to_string());
                put(&label, "INFOSTREAM", env.infostream.to_string());
                put(
                    &label,
                    "TEST_ASSERTS_ENABLED",
                    env.test_asserts_enabled.to_string(),
                );
                put(&label, "TEST_CODEC", env.test_codec.clone());
                put(
                    &label,
                    "TEST_POSTINGSFORMAT",
                    env.test_postingsformat.clone(),
                );
                put(
                    &label,
                    "TEST_DOCVALUESFORMAT",
                    env.test_docvaluesformat.clone(),
                );
                put(&label, "TEST_DIRECTORY", env.test_directory.clone());
                put(
                    &label,
                    "TEST_LINE_DOCS_FILE",
                    env.test_line_docs_file.clone(),
                );
                put(&label, "TEST_NIGHTLY", env.test_nightly.to_string());
                put(&label, "TEST_WEEKLY", env.test_weekly.to_string());
                put(&label, "TEST_MONSTER", env.test_monster.to_string());
                put(&label, "TEST_AWAITSFIX", env.test_awaitsfix.to_string());
                put(&label, "TEST_THROTTLING", env.test_throttling.to_string());
                put(
                    &label,
                    "RANDOM_MULTIPLIER",
                    env.random_multiplier.to_string(),
                );
                put(&label, "LEAVE_TEMPORARY", env.leave_temporary.to_string());
                put(
                    &label,
                    "defaultRandomMultiplier",
                    if env.test_nightly { "2" } else { "1" }.into(),
                );
            }
            Err(p) => {
                let t = Throwable::from_panic(p);
                put(
                    &label,
                    "error",
                    format!("{}\t{}", t.class_name(), t.message().unwrap_or("<null>")),
                );
            }
        }
    }
    for p in PROPERTIES {
        system::clear_property(p);
    }
    let expected: BTreeMap<String, String> = rows("ltc_env.tsv")
        .into_iter()
        .map(|r| (format!("{}\t{}", r[0], r[1]), r[2..].join("\t")))
        .collect();
    assert_same("static environment", &expected, &actual);
}

fn row(rows: &mut BTreeMap<String, String>, label: &str, op: impl FnOnce() -> String) {
    let line = match catch_unwind(AssertUnwindSafe(op)) {
        Ok(r) => format!("OK\t{}", r.replace('\n', "\\n")),
        Err(p) => {
            let t = Throwable::from_panic(p);
            let cause = t.cause().map(|c| format!(" <- {c}")).unwrap_or_default();
            format!(
                "{}\t{}{}",
                t.class_name(),
                t.message()
                    .unwrap_or("<null>")
                    .replace('\n', "\\n")
                    .replace('\t', "\\t"),
                cause.replace('\n', "\\n")
            )
        }
    };
    rows.insert(label.to_string(), line);
}

fn throws(class: &str, message: &str) -> impl FnOnce() -> Result<(), Throwable> {
    let t = Throwable::new(class.to_string(), message.to_string());
    move || Err(t)
}

fn throws_caused(
    class: &str,
    message: &str,
    cause: Throwable,
) -> impl FnOnce() -> Result<(), Throwable> {
    let t = Throwable::with_cause(class.to_string(), Some(message.to_string()), cause);
    move || Err(t)
}

fn ok() -> impl FnOnce() -> Result<(), Throwable> {
    || Ok(())
}

#[test]
fn statics_match_lucene() {
    use randomizedtesting::jdk::exception::register_class;
    register_class("java.io.IOException", "java.lang.Exception");
    let r = &mut BTreeMap::new();
    let fs = [
        0f32,
        -0f32,
        1f32,
        f32::from_bits(1),
        -f32::from_bits(1),
        f32::NAN,
        f32::INFINITY,
        f32::MAX,
        1.000_000_1f32,
        0.1f32,
    ];
    for x in fs {
        for y in fs {
            for u in [0i16, 1, 2, 100] {
                row(
                    r,
                    &format!(
                        "assertFloatUlpEquals {:x} {:x} {u}",
                        x.to_bits(),
                        y.to_bits()
                    ),
                    || {
                        Ltc::assert_float_ulp_equals(x, y, u);
                        "null".into()
                    },
                );
            }
        }
    }
    let ds = [
        0f64,
        -0f64,
        1f64,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::NAN,
        f64::NEG_INFINITY,
        1.000_000_000_000_000_2f64,
        0.1f64,
        1e300f64,
    ];
    for x in ds {
        for y in ds {
            for u in [0i32, 1, 2, i32::MAX] {
                row(
                    r,
                    &format!(
                        "assertDoubleUlpEquals {:x} {:x} {u}",
                        x.to_bits(),
                        y.to_bits()
                    ),
                    || {
                        Ltc::assert_double_ulp_equals(x, y, u);
                        "null".into()
                    },
                );
            }
        }
    }
    let ise = "java.lang.IllegalStateException";
    let iae = "java.lang.IllegalArgumentException";
    let ioe = "java.io.IOException";
    let re = "java.lang.RuntimeException";
    let ae = "java.lang.AssertionError";
    row(r, "expectThrows match", || {
        Ltc::expect_throws(ise, throws(ise, "boom")).to_string()
    });
    row(r, "expectThrows subclass", || {
        Ltc::expect_throws(re, throws(iae, "sub")).to_string()
    });
    row(r, "expectThrows none", || {
        Ltc::expect_throws(ise, ok()).to_string()
    });
    row(r, "expectThrows wrong", || {
        Ltc::expect_throws(ise, throws(ioe, "io")).to_string()
    });
    row(r, "expectThrows assertion", || {
        Ltc::expect_throws(ise, throws(ae, "ae")).to_string()
    });
    row(r, "expectThrows assertion expected", || {
        Ltc::expect_throws(ae, throws(ae, "ae")).to_string()
    });
    row(r, "expectThrows msg none", || {
        Ltc::expect_throws_msg(ioe, "custom", ok()).to_string()
    });
    row(r, "expectThrowsAnyOf match", || {
        Ltc::expect_throws_any_of(&[ioe, ise], throws(ise, "x")).to_string()
    });
    row(r, "expectThrowsAnyOf none", || {
        Ltc::expect_throws_any_of(&[ioe, ise], ok()).to_string()
    });
    row(r, "expectThrowsAnyOf wrong", || {
        Ltc::expect_throws_any_of(&[ioe, ise], throws(iae, "y")).to_string()
    });
    row(r, "expectThrowsAnyOf empty", || {
        Ltc::expect_throws_any_of(&[], ok()).to_string()
    });
    let inner = |c: &str| Throwable::new(c.to_string(), "inner".to_string());
    row(r, "expectThrows wrapped match", || {
        Ltc::expect_throws_wrapped(re, ioe, throws_caused(re, "outer", inner(ioe))).to_string()
    });
    row(r, "expectThrows wrapped wrong cause", || {
        Ltc::expect_throws_wrapped(re, ioe, throws_caused(re, "outer", inner(ise))).to_string()
    });
    row(r, "expectThrows wrapped no cause", || {
        Ltc::expect_throws_wrapped(re, ioe, throws(re, "outer")).to_string()
    });
    row(r, "expectThrows wrapped wrong outer", || {
        Ltc::expect_throws_wrapped(ise, ioe, throws(re, "outer")).to_string()
    });
    row(r, "expectThrows wrapped none", || {
        Ltc::expect_throws_wrapped(re, ioe, ok()).to_string()
    });
    let map: &[(&str, &[&str])] = &[(ise, &[]), (re, &[ioe, iae])];
    let opt = |t: Option<Throwable>| t.map_or_else(|| "null".to_string(), |t| t.to_string());
    row(r, "expectThrowsAnyOf wrapped empty list", || {
        opt(Ltc::expect_throws_any_of_wrapped(map, throws(ise, "s")))
    });
    row(r, "expectThrowsAnyOf wrapped match", || {
        let i = Throwable::new(ioe.to_string(), "i".to_string());
        opt(Ltc::expect_throws_any_of_wrapped(
            map,
            throws_caused(re, "r", i),
        ))
    });
    row(r, "expectThrowsAnyOf wrapped wrong cause", || {
        let i = Throwable::new(ise.to_string(), "i".to_string());
        opt(Ltc::expect_throws_any_of_wrapped(
            map,
            throws_caused(re, "r", i),
        ))
    });
    row(r, "expectThrowsAnyOf wrapped wrong outer", || {
        opt(Ltc::expect_throws_any_of_wrapped(
            map,
            throws("java.lang.Exception", "e"),
        ))
    });
    row(r, "expectThrowsAnyOf wrapped none", || {
        opt(Ltc::expect_throws_any_of_wrapped(map, ok()))
    });
    row(r, "assertThat ok", || {
        Ltc::assert_that(&5i32, &equal_to(5i32));
        "null".into()
    });
    row(r, "assertThat int", || {
        Ltc::assert_that(&5i32, &equal_to(6i32));
        "null".into()
    });
    row(r, "assertThat is string", || {
        Ltc::assert_that(&"a\"b".to_string(), &is(equal_to("c\td".to_string())));
        "null".into()
    });
    row(r, "assertThat reason long", || {
        Ltc::assert_that_reason("why", &5i64, &is(equal_to(6i64)));
        "null".into()
    });
    row(r, "assertThat float", || {
        Ltc::assert_that(&1.5f32, &equal_to(2f32));
        "null".into()
    });
    row(r, "assertThat double", || {
        Ltc::assert_that(&1.5f64, &equal_to(1e-10f64));
        "null".into()
    });
    row(r, "assertThat bool", || {
        Ltc::assert_that(&true, &is(equal_to(false)));
        "null".into()
    });
    row(r, "dumpArray", || {
        let mut b = Vec::new();
        let objs: [&dyn std::fmt::Display; 3] = [&1, &"two", &"3.0"];
        let objs: Vec<String> = objs.iter().map(|o| o.to_string()).collect();
        Ltc::dump_array("lbl", Some(&objs[..]), &mut b).unwrap();
        String::from_utf8(b).unwrap()
    });
    row(r, "dumpArray null", || {
        let mut b = Vec::new();
        Ltc::dump_array::<String>("lbl", None, &mut b).unwrap();
        String::from_utf8(b).unwrap()
    });
    row(r, "dumpIterator empty", || {
        let mut b = Vec::new();
        Ltc::dump_iterator("e", Some(std::iter::empty::<String>()), &mut b).unwrap();
        String::from_utf8(b).unwrap()
    });
    row(r, "asSet", || {
        let mut v: Vec<i32> = Ltc::as_set([3, 1, 3, 2]).into_iter().collect();
        v.sort();
        format!(
            "[{}]",
            v.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    row(r, "assumeTrue", || {
        Ltc::assume_true("msg", false);
        "null".into()
    });
    row(r, "assumeFalse", || {
        Ltc::assume_false("msg", true);
        "null".into()
    });
    row(r, "assumeNoException", || {
        Ltc::assume_no_exception(
            "msg",
            Some(&Throwable::new(ioe.to_string(), "x".to_string())),
        );
        "null".into()
    });
    row(r, "TestUtil.nextInt range", || {
        let rnd = JdkRandom::new(1);
        let v: Vec<String> = (0..8)
            .map(|_| TestUtil::next_int(&rnd, -3, 9).to_string())
            .collect();
        format!("[{}]", v.join(", "))
    });
    row(r, "TestUtil.nextInt bad", || {
        TestUtil::next_int(&JdkRandom::new(1), 5, 3).to_string()
    });
    {
        let _guard = PROPS.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = oracle_support::TempDir::new("fork");
        let file = tmp.0.join("fork.args");
        std::fs::write(&file, "-Da=1\n-Xmx1g\r\n-ea").unwrap();
        let fork = || format!("[{}]", Ltc::get_jvm_fork_arguments().unwrap().join(", "));
        system::clear_property("tests.jvmForkArgsFile");
        row(r, "getJvmForkArguments unset", fork);
        system::set_property("tests.jvmForkArgsFile", &file.to_string_lossy());
        row(r, "getJvmForkArguments file", fork);
        system::set_property(
            "tests.jvmForkArgsFile",
            &format!("{}.missing", file.to_string_lossy()),
        );
        row(r, "getJvmForkArguments missing", fork);
        system::clear_property("tests.jvmForkArgsFile");
    }
    row(r, "getTestClass outside", || {
        Ltc::get_test_class().name().to_string()
    });

    let expected: BTreeMap<String, String> = labelled("ltc_statics.tsv");
    assert_same("statics", &expected, r);
}

#[test]
fn locale_for_language_tag_matches_lucene() {
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in rows("ltc_locales.tsv") {
        let units = r[0].clone();
        expected.insert(units.clone(), r[1..].join("\t"));
        let tag = from_units(&units);
        let line = match catch_unwind(AssertUnwindSafe(|| Ltc::locale_for_language_tag(&tag))) {
            Ok(l) => format!("OK\t{}|{}", l, l.to_language_tag()),
            Err(p) => {
                let t = Throwable::from_panic(p);
                format!("{}\t{}", t.class_name(), t.message().unwrap_or("<null>"))
            }
        };
        actual.insert(units, line);
    }
    assert_same("localeForLanguageTag", &expected, &actual);
}

fn from_units(units: &str) -> String {
    if units == "-" {
        return String::new();
    }
    let u: Vec<u16> = units
        .split('.')
        .map(|x| u16::from_str_radix(x, 16).unwrap())
        .collect();
    String::from_utf16(&u).unwrap()
}

fn br(b: &BytesRef) -> String {
    let hex: String = b.bytes.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}/{}/{}/{hex}", b.offset, b.length, b.bytes.len())
}

/// The body of `TestLtcDraws.testDraws`.
fn test_draws() -> String {
    let mut sb = String::new();
    sb.push_str(&format!("class={}", Ltc::get_test_class().name()));
    for i in [1, 2, 3, 5, 10, 100, 1000] {
        sb.push_str(&format!(" atLeast({i})={}", Ltc::at_least(i)));
    }
    sb.push_str(&format!(
        " atLeastR={}",
        Ltc::at_least_with(&*Ltc::random(), 7)
    ));
    sb.push_str(" rarely=");
    for _ in 0..64 {
        sb.push(if Ltc::rarely() { '1' } else { '0' });
    }
    sb.push_str(" usually=");
    for _ in 0..64 {
        sb.push(if Ltc::usually() { '1' } else { '0' });
    }
    sb.push_str(" maybeCache=");
    let query = rucene::search::MatchAllDocsQuery::instance();
    for _ in 0..16 {
        sb.push(if MAYBE_CACHE_POLICY.should_cache(&query).unwrap() {
            '1'
        } else {
            '0'
        });
    }
    for _ in 0..12 {
        sb.push_str(&format!(" bytes={}", br(&Ltc::new_bytes_ref_str("hello"))));
    }
    for _ in 0..4 {
        sb.push_str(&format!(" empty={}", br(&Ltc::new_bytes_ref())));
    }
    for _ in 0..4 {
        sb.push_str(&format!(" len3={}", br(&Ltc::new_bytes_ref_len(3))));
    }
    for _ in 0..4 {
        sb.push_str(&format!(
            " slice={}",
            br(&Ltc::new_bytes_ref_slice(&[1, 2, 3, 4, 5], 1, 3))
        ));
    }
    for _ in 0..4 {
        let b = BytesRef {
            bytes: vec![9, 8, 7],
            offset: 1,
            length: 2,
        };
        sb.push_str(&format!(" copy={}", br(&Ltc::new_bytes_ref_from(&b))));
    }
    for _ in 0..12 {
        let a = Ltc::random_locale(&*Ltc::random());
        let b = Ltc::random_locale(&*Ltc::random());
        sb.push_str(&format!(" locale={a}|{}", b.to_language_tag()));
    }
    for _ in 0..12 {
        sb.push_str(&format!(
            " tz={}",
            Ltc::random_time_zone(&*Ltc::random()).get_id()
        ));
    }
    sb.push_str(&format!(" last={}", Ltc::random().next_long()));
    sb
}

static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[test]
fn random_helpers_match_lucene_under_the_runner() {
    let _guard = PROPS.lock().unwrap_or_else(|e| e.into_inner());
    let class = Class::builder("org.apache.lucene.tests.util.TestLtcDraws")
        .constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("classNameRule", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::clone(&CLASS_NAME_RULE) as Arc<dyn randomizedtesting::junit::TestRule>
                )
            })
            .annotate(ClassRule),
        )
        .method(Method::new("testDraws").annotate(Test::new()).body_any(|| {
            let d = test_draws();
            DRAWS.lock().unwrap().push(d);
        }))
        .build();
    let class = Arc::new(class);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in rows("ltc_draws.tsv") {
        expected.insert(r[0].clone(), r[1].clone());
    }
    for seed in expected.keys() {
        DRAWS.lock().unwrap().clear();
        system::set_property("tests.seed", seed);
        let result = JUnitCore::new().run(&Request::a_class(&class));
        if let Some(f) = result.failures().first() {
            panic!("seed {seed}: {}", f.exception());
        }
        actual.insert(seed.clone(), DRAWS.lock().unwrap().join(""));
    }
    system::clear_property("tests.seed");
    assert_same("random helpers", &expected, &actual);
}
