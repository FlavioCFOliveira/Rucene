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
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{JUnitCore, Request};
use rucene::util::BytesRef;
use rucene_test_framework::hamcrest::{equal_to, is};
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase as Ltc, StaticEnvironment, TimeZone, MAYBE_CACHE_POLICY,
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
    // `TestLtcDraws extends LuceneTestCase`: the class and test rule chains
    // are `LuceneTestCase`'s, and `LuceneJUnit3MethodProvider` finds
    // `testDraws`.
    let class = Class::builder("org.apache.lucene.tests.util.TestLtcDraws")
        .extends(&Ltc::class())
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("testDraws").body_any(|| {
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

// ---------------------------------------------------------------------------
// Task #349: custom time-zone IDs and the remaining members
// (`tests/fixtures/ltc_members.tsv`, `LuceneTestCaseOracle.members`).
// ---------------------------------------------------------------------------

/// `TimeZone.getTimeZone(id)` parses custom `GMT±h[h][:mm]` IDs as JDK 25
/// does: the normalized ID and the raw offset of every ID of the oracle's
/// grid (prefixes × signs × bodies), and `GMT` for the IDs it rejects.
#[test]
fn custom_time_zone_ids_match_lucene() {
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for r in rows("ltc_members.tsv").into_iter().filter(|r| r[0] == "tz") {
        expected.insert(r[1].clone(), format!("{}\t{}", r[2], r[3]));
        let tz = TimeZone::get_time_zone(&r[1]);
        actual.insert(
            r[1].clone(),
            format!("{}\t{}", tz.get_id(), tz.get_raw_offset()),
        );
    }
    assert_eq!(expected.len(), 576);
    assert_same("custom time zones", &expected, &actual);
}

/// `TestLtcMembers.check`.
fn check(sb: &mut String, label: &str, op: impl FnOnce()) {
    sb.push_str(&format!(" {label}="));
    match catch_unwind(AssertUnwindSafe(op)) {
        Ok(()) => sb.push_str("ok"),
        Err(p) => {
            let t = Throwable::from_panic(p);
            sb.push_str(&format!(
                "{}:{}",
                t.class_name(),
                t.message().unwrap_or("null").replace('\n', "\\n")
            ));
        }
    }
}

/// `TestLtcMembers.reader`.
fn members_reader(
    ids: &[&str],
    ns: &[i64],
    ps: &[i32],
    extra: bool,
    delete: Option<&str>,
) -> Arc<dyn rucene::index::IndexReader> {
    use rucene::analysis::standard::StandardAnalyzer;
    use rucene::document::{Document, IntPoint, NumericDocValuesField, Store, StringField};
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::index::terms::Term;
    use rucene::store::{ByteBuffersDirectory, Directory};
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new()))),
    )
    .unwrap();
    for i in 0..ids.len() {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", ids[i].to_string(), Store::YES).unwrap(),
        ));
        d.add(Box::new(NumericDocValuesField::new("n", ns[i])));
        d.add(Box::new(IntPoint::new("p", &[ps[i]]).unwrap()));
        if extra {
            d.add(Box::new(
                StringField::new("x", "y".to_string(), Store::NO).unwrap(),
            ));
        }
        w.add_document(&d).unwrap();
    }
    if let Some(delete) = delete {
        w.delete_documents_by_terms(vec![Term::new("id", BytesRef::from_char_sequence(delete))])
            .unwrap();
    }
    w.close().unwrap();
    rucene::index::directory_reader::open(dir).unwrap() as Arc<dyn rucene::index::IndexReader>
}

/// The body of `TestLtcMembers.testMembers`.
fn test_members() -> String {
    use rucene::document::{FieldType, Store, StringField, TextField};
    use rucene::index::IndexOptions;
    use rucene_test_framework::util::lucene_test_case::NewFieldValue;
    let mut sb = String::new();
    sb.push_str(&format!(
        "name={} thread={}",
        Ltc::get_test_name().unwrap_or_else(|| "null".to_string()),
        Ltc::is_test_thread()
    ));
    for i in 0..6 {
        let store = if i % 2 == 0 { Store::YES } else { Store::NO };
        let f = Ltc::new_string_field(&format!("s{}", i % 3), "v", store);
        sb.push_str(&format!(" ns={}", f.field_type()));
    }
    for i in 0..6 {
        let store = if i % 2 == 0 { Store::YES } else { Store::NO };
        let f = Ltc::new_text_field(&format!("t{}", i % 4), "some text", store);
        sb.push_str(&format!(" nt={}", f.field_type()));
    }
    for i in 0..4 {
        let f = Ltc::new_string_field_bytes_with(
            &*Ltc::random(),
            &format!("b{i}"),
            BytesRef::new(vec![1, 2]),
            Store::NO,
        );
        sb.push_str(&format!(
            " nb={}|{}",
            f.field_type(),
            f.binary_value().unwrap()
        ));
    }
    let custom = FieldType::new();
    custom
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    custom.set_tokenized(false).unwrap();
    for i in 0..4 {
        sb.push_str(&format!(
            " nf={}",
            Ltc::new_field(&format!("c{i}"), "x", &custom).field_type()
        ));
        sb.push_str(&format!(
            " nfs={}",
            Ltc::new_field(&format!("d{i}"), "x", StringField::type_stored()).field_type()
        ));
        sb.push_str(&format!(
            " nft={}",
            Ltc::new_field_with(
                &*Ltc::random(),
                &format!("e{i}"),
                NewFieldValue::String("x".to_string()),
                TextField::type_not_stored()
            )
            .field_type()
        ));
    }
    sb.push_str(&format!(" lmp={}", Ltc::new_log_merge_policy_use_cfs(true)));
    sb.push_str(&format!(
        " lmp2={}",
        Ltc::new_log_merge_policy_use_cfs_merge_factor(false, 7).unwrap()
    ));
    sb.push_str(&format!(
        " lmp3={}",
        Ltc::new_log_merge_policy_merge_factor(9).unwrap()
    ));
    sb.push_str(&format!(" tmp={}", Ltc::new_tiered_merge_policy()));
    sb.push_str(&format!(" lmp4={}", Ltc::new_log_merge_policy()));

    let l = members_reader(&["a", "b"], &[1, 2], &[1, 2], false, None);
    let l2 = members_reader(&["a", "b"], &[1, 2], &[1, 2], false, None);
    let one = members_reader(&["a"], &[1], &[1], false, None);
    let ids = members_reader(&["a", "c"], &[1, 2], &[1, 2], false, None);
    let dvs = members_reader(&["a", "b"], &[1, 3], &[1, 2], false, None);
    let extra = members_reader(&["a", "b"], &[1, 2], &[1, 2], true, None);
    let pts = members_reader(&["a", "b"], &[1, 2], &[1, 5], false, None);
    let del_a = members_reader(&["a", "b"], &[1, 2], &[1, 2], false, Some("a"));
    let del_b = members_reader(&["a", "b"], &[1, 2], &[1, 2], false, Some("b"));
    check(&mut sb, "same", || {
        Ltc::assert_reader_equals("info", &l, &l2)
    });
    check(&mut sb, "stats", || {
        Ltc::assert_reader_statistics_equals("info", &l, &one)
    });
    check(&mut sb, "stored", || {
        Ltc::assert_stored_fields_equals("info", &l, &ids)
    });
    check(&mut sb, "terms", || {
        Ltc::assert_terms_equals("info", &l, &ids, true)
    });
    check(&mut sb, "dv", || {
        Ltc::assert_doc_values_equals("info", &l, &dvs)
    });
    check(&mut sb, "fieldInfos", || {
        Ltc::assert_field_infos_equals("info", &l, &extra)
    });
    check(&mut sb, "termsFields", || {
        Ltc::assert_terms_equals("info", &l, &extra, false)
    });
    check(&mut sb, "norms", || {
        Ltc::assert_norms_equals("info", &l, &extra)
    });
    check(&mut sb, "dvFields", || {
        Ltc::assert_doc_values_equals("info", &extra, &l)
    });
    check(&mut sb, "points", || {
        Ltc::assert_points_equals("info", &l, &pts)
    });
    check(&mut sb, "deleted", || {
        Ltc::assert_deleted_docs_equals("info", &del_a, &del_b)
    });
    check(&mut sb, "deletedCount", || {
        Ltc::assert_deleted_docs_equals("info", &l, &del_b)
    });
    check(&mut sb, "tv", || {
        Ltc::assert_term_vectors_equals("info", &l, &ids)
    });
    for r in [l, l2, one, ids, dvs, extra, pts, del_a, del_b] {
        r.close().unwrap();
    }
    sb.push_str(&format!(" last={}", Ltc::random().next_long()));
    sb
}

/// The remaining `LuceneTestCase` members (`newField` family, merge-policy
/// overloads, `getTestName`, `isTestThread`, the `assert*Equals` family on
/// equal and deliberately different readers) draw and fail as Java's do in
/// `TestLtcMembers extends LuceneTestCase`, for every seed of the oracle.
#[test]
fn members_match_lucene_under_the_runner() {
    static MEMBERS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let _guard = PROPS.lock().unwrap_or_else(|e| e.into_inner());
    let class = Arc::new(
        Class::builder("org.apache.lucene.tests.util.TestLtcMembers")
            .extends(&Ltc::class())
            .constructor(Constructor::no_args(|| ()))
            .method(Method::new("testMembers").body_any(|| {
                let d = test_members();
                MEMBERS.lock().unwrap().push(d);
            }))
            .build(),
    );
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in rows("ltc_members.tsv")
        .into_iter()
        .filter(|r| r[0] == "members")
    {
        expected.insert(r[1].clone(), r[2].clone());
    }
    assert_eq!(expected.len(), 8);
    for seed in expected.keys() {
        MEMBERS.lock().unwrap().clear();
        system::set_property("tests.seed", seed);
        let result = JUnitCore::new().run(&Request::a_class(&class));
        if let Some(f) = result.failures().first() {
            panic!("seed {seed}: {}\n{}", f.exception(), f.trace());
        }
        actual.insert(seed.clone(), MEMBERS.lock().unwrap().join(""));
    }
    system::clear_property("tests.seed");
    assert_same("members", &expected, &actual);
}

/// `TestLtcDirs.describe`: the directory's `toString()` (the temporary root
/// as `$D`, identity hashes as `@HASH`), then the outcome of a second
/// `obtainLock` of a held lock, after which the directory is closed.
fn describe_dir(
    mut d: rucene_test_framework::util::lucene_test_case::TestDirectory,
    masks: &[String],
) -> String {
    let mut s = d.to_string();
    for m in masks {
        s = s.replace(m.as_str(), "$D");
    }
    let s = oracle_support::mask(&s, "\u{0}", None);
    let dir = d.as_directory();
    let again = {
        let l = dir.obtain_lock("t.lock").unwrap();
        let again = match dir.obtain_lock("t.lock") {
            Ok(l2) => {
                l2.close().unwrap();
                "again".to_string()
            }
            Err(e) => e.java_simple_name().to_string(),
        };
        l.close().unwrap();
        again
    };
    drop(dir);
    d.close().unwrap();
    format!("{s}|{again}")
}

/// `TestLtcDirs.testDirs`.
fn test_dirs() -> String {
    use rucene::index::VectorEncoding;
    use rucene::store::{
        LockFactory, NativeFSLockFactory, NoLockFactory, SimpleFSLockFactory,
        SingleInstanceLockFactory,
    };
    use rucene_test_framework::util::lucene_test_case::TestDirectory;
    let lock_factories = || -> Vec<Arc<dyn LockFactory>> {
        vec![
            Arc::new(*NoLockFactory::instance()),
            Arc::new(SingleInstanceLockFactory::new()),
            Arc::new(*NativeFSLockFactory::instance()),
            Arc::new(*SimpleFSLockFactory::instance()),
        ]
    };
    let fs_lock_factories = || -> Vec<Arc<dyn LockFactory>> {
        vec![
            Arc::new(*NativeFSLockFactory::instance()),
            Arc::new(*SimpleFSLockFactory::instance()),
            Arc::new(*NoLockFactory::instance()),
        ]
    };
    let mut sb = String::new();
    let base = Ltc::create_temp_dir_prefix("base").get_parent().unwrap();
    let masks = [
        base.to_real_path(&[]).unwrap().to_string(),
        base.to_string(),
    ];
    for _ in 0..3 {
        for lf in lock_factories() {
            let d = Ltc::new_directory_lock_factory(&*Ltc::random(), Arc::clone(&lf));
            sb.push_str(&format!(" nd={}", describe_dir(d, &masks)));
            let m = Ltc::new_mock_directory_lock_factory(&*Ltc::random(), lf);
            sb.push_str(&format!(
                " nmd={}",
                describe_dir(TestDirectory::Mock(m), &masks)
            ));
        }
    }
    for lf in fs_lock_factories() {
        let d =
            Ltc::new_fs_directory_lock_factory(&Ltc::create_temp_dir_prefix("fs"), Arc::clone(&lf));
        sb.push_str(&format!(" nfs={}", describe_dir(d, &masks)));
        let m = Ltc::new_mock_fs_directory(&Ltc::create_temp_dir_prefix("mfs"));
        sb.push_str(&format!(
            " nmfs={}",
            describe_dir(TestDirectory::Mock(m), &masks)
        ));
        let m = Ltc::new_mock_fs_directory_lock_factory(&Ltc::create_temp_dir_prefix("mfsl"), lf);
        sb.push_str(&format!(
            " nmfsl={}",
            describe_dir(TestDirectory::Mock(m), &masks)
        ));
    }
    for (label, e) in [
        ("BYTE", VectorEncoding::BYTE),
        ("FLOAT32", VectorEncoding::FLOAT32),
    ] {
        let names: Vec<String> = (0..12)
            .map(|_| Ltc::random_vector_format(e).name().to_string())
            .collect();
        sb.push_str(&format!(" rvf{label}={}", names.join(",")));
    }
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let not_found = |e: rucene::error::LuceneError| match e {
        rucene::error::LuceneError::NotFound(m) => format!("java.io.FileNotFoundException:{m}"),
        other => format!("unexpected:{other}"),
    };
    for name in ["nope.txt", "/nope.txt"] {
        match Ltc::get_data_path(&resources, name) {
            Ok(_) => sb.push_str(" dp=found"),
            Err(e) => sb.push_str(&format!(" dp={}", not_found(e))),
        }
        match Ltc::get_data_input_stream(&resources, name) {
            Ok(_) => sb.push_str(" dis=found"),
            Err(e) => sb.push_str(&format!(" dis={}", not_found(e))),
        }
    }
    sb.push_str(&format!(" last={}", Ltc::random().next_long()));
    sb
}

/// The lock-factory `newDirectory`/`newMockDirectory`/`newFSDirectory`
/// overloads, `newMockFSDirectory` with and without a lock factory,
/// `randomVectorFormat` and the missing-resource errors of
/// `getDataPath`/`getDataInputStream` draw, render and lock as Java's do in
/// `TestLtcDirs extends LuceneTestCase`, for every seed of the oracle.
#[test]
fn directories_and_vector_formats_match_lucene_under_the_runner() {
    static DIRS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let _guard = PROPS.lock().unwrap_or_else(|e| e.into_inner());
    let class = Arc::new(
        Class::builder("org.apache.lucene.tests.util.TestLtcDirs")
            .extends(&Ltc::class())
            .constructor(Constructor::no_args(|| ()))
            .method(Method::new("testDirs").body_any(|| {
                let d = test_dirs();
                DIRS.lock().unwrap().push(d);
            }))
            .build(),
    );
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in rows("ltc_members.tsv")
        .into_iter()
        .filter(|r| r[0] == "dirs")
    {
        expected.insert(r[1].clone(), r[2].clone());
    }
    assert_eq!(expected.len(), 8);
    for seed in expected.keys() {
        DIRS.lock().unwrap().clear();
        system::set_property("tests.seed", seed);
        let result = JUnitCore::new().run(&Request::a_class(&class));
        if let Some(f) = result.failures().first() {
            panic!("seed {seed}: {}\n{}", f.exception(), f.trace());
        }
        actual.insert(seed.clone(), DIRS.lock().unwrap().join(""));
    }
    system::clear_property("tests.seed");
    assert_same("dirs", &expected, &actual);
}

/// `getDataPath`/`getDataInputStream` find a resource that exists under the
/// directory standing for the test class's package, with or without a
/// leading `/`.
#[test]
fn data_path_finds_an_existing_resource() {
    use std::io::Read;
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["ltc_members.tsv", "/ltc_members.tsv"] {
        assert_eq!(
            Ltc::get_data_path(&resources, name).unwrap(),
            resources.join("ltc_members.tsv")
        );
        let mut head = [0u8; 4];
        Ltc::get_data_input_stream(&resources, name)
            .unwrap()
            .read_exact(&mut head)
            .unwrap();
        assert_eq!(&head, b"# tz");
    }
}
