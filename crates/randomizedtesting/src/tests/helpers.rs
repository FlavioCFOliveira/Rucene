//! `RandomizedTest` helpers under a context (`helpers.tsv`): every helper,
//! 20 private seeds × 200 calls, under five property configurations
//! (`tests.nightly`, `randomized.multiplier`), plus their errors, the
//! assumptions and `SysGlobals`.

#![allow(deprecated)]

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use super::{assert_same_lines, catch, java_error, rows, seeds, Rec};
use crate::group_evaluator::GroupEvaluator;
use crate::jdk::system;
use crate::jdk::thread::{Thread, ThreadGroup};
use crate::random_supplier::default_supplier;
use crate::{RandomizedContext, RandomizedTest as T, Randomness, SysGlobals};

const HELPER_DRAWS: usize = 200;

type Draw = Box<dyn Fn(&mut Rec)>;
type Call = Box<dyn Fn()>;

fn helper_methods() -> Vec<(String, Draw)> {
    let mut m: Vec<(String, Draw)> = vec![
        (
            "randomBoolean".into(),
            Box::new(|rec| rec.z(T::random_boolean())),
        ),
        (
            "randomByte".into(),
            Box::new(|rec| rec.i(i32::from(T::random_byte()))),
        ),
        (
            "randomShort".into(),
            Box::new(|rec| rec.i(i32::from(T::random_short()))),
        ),
        ("randomInt".into(), Box::new(|rec| rec.i(T::random_int()))),
        (
            "randomFloat".into(),
            Box::new(|rec| rec.f(T::random_float())),
        ),
        (
            "randomDouble".into(),
            Box::new(|rec| rec.d(T::random_double())),
        ),
        ("randomLong".into(), Box::new(|rec| rec.l(T::random_long()))),
        (
            "randomGaussian".into(),
            Box::new(|rec| rec.d(T::random_gaussian())),
        ),
        (
            "biasedFloatBetween(-1,1)".into(),
            Box::new(|rec| rec.f(T::biased_float_between(-1.0, 1.0))),
        ),
        (
            "biasedDoubleBetween(-inf,inf)".into(),
            Box::new(|rec| rec.d(T::biased_double_between(f64::NEG_INFINITY, f64::INFINITY))),
        ),
        (
            "randomBytesOfLength(9)".into(),
            Box::new(|rec| rec.bytes(&T::random_bytes_of_length(9))),
        ),
        (
            "randomBytesOfLength(0,20)".into(),
            Box::new(|rec| rec.bytes(&T::random_bytes_of_length_between(0, 20))),
        ),
        (
            "randomInt(10)".into(),
            Box::new(|rec| rec.i(T::random_int_max(10))),
        ),
        (
            "randomLong(10)".into(),
            Box::new(|rec| rec.l(T::random_long_max(10))),
        ),
        (
            "randomIntBetween(-7,7)".into(),
            Box::new(|rec| rec.i(T::random_int_between(-7, 7))),
        ),
        (
            "between(int)(3,9)".into(),
            Box::new(|rec| rec.i(T::between(3, 9))),
        ),
        (
            "randomLongBetween(-7,7)".into(),
            Box::new(|rec| rec.l(T::random_long_between(-7, 7))),
        ),
        (
            "between(long)(min,max)".into(),
            Box::new(|rec| rec.l(T::between_long(i64::MIN, i64::MAX))),
        ),
    ];
    for k in [0, 1, 10, 1000, i32::MAX] {
        m.push((
            format!("atLeast({k})"),
            Box::new(move |rec| rec.i(T::at_least(k))),
        ));
        m.push((
            format!("atMost({k})"),
            Box::new(move |rec| rec.i(T::at_most(k))),
        ));
    }
    m.push(("rarely".into(), Box::new(|rec| rec.z(T::rarely()))));
    m.push(("frequently".into(), Box::new(|rec| rec.z(T::frequently()))));
    m.push((
        "randomFrom(Object[])".into(),
        Box::new(|rec| rec.i(*T::random_from(&[1, 2, 3, 4, 5]))),
    ));
    m.push((
        "randomFrom(List)".into(),
        Box::new(|rec| rec.i(*T::random_from_list(&[1, 2, 3, 4, 5]))),
    ));
    m.push((
        "randomFrom(byte[])".into(),
        Box::new(|rec| rec.i(i32::from(*T::random_from(&[1i8, 2, 3])))),
    ));
    m.push((
        "randomFrom(short[])".into(),
        Box::new(|rec| rec.i(i32::from(*T::random_from(&[1i16, 2, 3])))),
    ));
    m.push((
        "randomFrom(int[])".into(),
        Box::new(|rec| rec.i(*T::random_from(&[1, 2, 3]))),
    ));
    m.push((
        "randomFrom(char[])".into(),
        Box::new(|rec| {
            rec.i(i32::from(*T::random_from(&[
                b'x' as u16,
                b'y' as u16,
                b'z' as u16,
            ])))
        }),
    ));
    m.push((
        "randomFrom(float[])".into(),
        Box::new(|rec| rec.f(*T::random_from(&[1f32, 2.0, 3.0]))),
    ));
    m.push((
        "randomFrom(long[])".into(),
        Box::new(|rec| rec.l(*T::random_from(&[1i64, 2, 3]))),
    ));
    m.push((
        "randomFrom(double[])".into(),
        Box::new(|rec| rec.d(*T::random_from(&[1f64, 2.0, 3.0]))),
    ));
    m.push(("multiplier".into(), Box::new(|rec| rec.d(T::multiplier()))));
    m.push(("isNightly".into(), Box::new(|rec| rec.z(T::is_nightly()))));
    m.push((
        "iterations(1,100)".into(),
        Box::new(|rec| rec.i(T::iterations(1, 100))),
    ));
    for (a, b) in [
        (0, 0),
        (0, 10),
        (5, 1000),
        (0, i32::MAX),
        (i32::MAX, i32::MAX),
    ] {
        m.push((
            format!("scaledRandomIntBetween({a},{b})"),
            Box::new(move |rec| rec.i(T::scaled_random_int_between(a, b))),
        ));
    }
    m.push((
        "randomLocale".into(),
        Box::new(|rec| rec.s(&T::random_locale().to_string())),
    ));
    m.push((
        "randomTimeZone".into(),
        Box::new(|rec| rec.s(T::random_time_zone().id())),
    ));
    m.push((
        "randomAsciiOfLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_of_length_between(0, 10))),
    ));
    m.push((
        "randomAsciiOfLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_of_length(6))),
    ));
    m.push((
        "randomAsciiLettersOfLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_letters_of_length_between(0, 10))),
    ));
    m.push((
        "randomAsciiLettersOfLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_letters_of_length(6))),
    ));
    m.push((
        "randomAsciiAlphanumOfLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_alphanum_of_length_between(0, 10))),
    ));
    m.push((
        "randomAsciiAlphanumOfLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_ascii_alphanum_of_length(6))),
    ));
    m.push((
        "randomUnicodeOfLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_unicode_of_length_between(0, 10))),
    ));
    m.push((
        "randomUnicodeOfLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_unicode_of_length(6))),
    ));
    m.push((
        "randomUnicodeOfCodepointLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_unicode_of_codepoint_length_between(0, 10))),
    ));
    m.push((
        "randomUnicodeOfCodepointLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_unicode_of_codepoint_length(6))),
    ));
    m.push((
        "randomRealisticUnicodeOfLengthBetween(0,10)".into(),
        Box::new(|rec| rec.s(&T::random_realistic_unicode_of_length_between(0, 10))),
    ));
    m.push((
        "randomRealisticUnicodeOfLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_realistic_unicode_of_length(6))),
    ));
    m.push((
        "randomRealisticUnicodeOfCodepointLengthBetween(0,10)".into(),
        Box::new(|rec| {
            rec.s(&T::random_realistic_unicode_of_codepoint_length_between(
                0, 10,
            ))
        }),
    ));
    m.push((
        "randomRealisticUnicodeOfCodepointLength(6)".into(),
        Box::new(|rec| rec.s(&T::random_realistic_unicode_of_codepoint_length(6))),
    ));
    m
}

fn outcome(f: impl FnOnce()) -> String {
    match catch(f) {
        Ok(()) => "OK\t-".into(),
        Err(p) => java_error(&p),
    }
}

/// The oracle's `HelpersSuite.testHelpers` for one configuration.
fn run_config(config: &str, log: &mut Vec<(String, String)>) {
    let methods = helper_methods();
    for seed in seeds() {
        for (label, draw) in &methods {
            let line = RandomizedContext::current().run_with_private_randomness_seed(seed, || {
                let mut rec = Rec::default();
                for _ in 0..HELPER_DRAWS {
                    draw(&mut rec);
                }
                rec.line()
            });
            log.push((format!("{config}\t{seed:x}\t{label}"), line));
        }
    }
    let errors: Vec<(&str, Call)> = vec![
        (
            "atLeast(-1)",
            Box::new(|| {
                T::at_least(-1);
            }),
        ),
        (
            "atMost(-1)",
            Box::new(|| {
                T::at_most(-1);
            }),
        ),
        (
            "scaledRandomIntBetween(-1,5)",
            Box::new(|| {
                T::scaled_random_int_between(-1, 5);
            }),
        ),
        (
            "scaledRandomIntBetween(5,4)",
            Box::new(|| {
                T::scaled_random_int_between(5, 4);
            }),
        ),
        (
            "randomFrom(Object[0])",
            Box::new(|| {
                T::random_from::<i32>(&[]);
            }),
        ),
        (
            "randomFrom(List[0])",
            Box::new(|| {
                T::random_from_list::<i32>(&[]);
            }),
        ),
    ];
    for (call, f) in errors {
        log.push((format!("{config}\terror\t{call}"), outcome(f)));
    }
    if config == "default" {
        let io = "java.io.IOException: io";
        let a: &dyn Display = &"a";
        let assumes: Vec<(&str, Call, &str)> = vec![
            ("assumeTrue(false)", Box::new(|| T::assume_true(false)), "-"),
            ("assumeFalse(true)", Box::new(|| T::assume_false(true)), "-"),
            (
                "assumeTrue(msg,false)",
                Box::new(|| T::assume_true_msg("the message", false)),
                "-",
            ),
            (
                "assumeFalse(msg,true)",
                Box::new(|| T::assume_false_msg("the message", true)),
                "-",
            ),
            (
                "assumeNotNull(a,null)",
                Box::new(move || T::assume_not_null(Some(&[Some(a), None]))),
                "-",
            ),
            (
                "assumeNotNull()",
                Box::new(|| T::assume_not_null(Some(&[]))),
                "-",
            ),
            (
                "assumeNotNull(a)",
                Box::new(move || T::assume_not_null(Some(&[Some(a)]))),
                "-",
            ),
            (
                "assumeNoException(msg,ex)",
                Box::new(move || T::assume_no_exception_msg("the message", Some(&io))),
                "java.io.IOException\tio",
            ),
            (
                "assumeNoException(ex)",
                Box::new(move || T::assume_no_exception(Some(&io))),
                "java.io.IOException\tio",
            ),
            (
                "assumeNoException(null)",
                Box::new(|| T::assume_no_exception(None)),
                "-",
            ),
            ("assumeTrue(true)", Box::new(|| T::assume_true(true)), "-"),
            (
                "assumeNotNull(null-array)",
                Box::new(|| T::assume_not_null(None)),
                "-",
            ),
        ];
        for (call, f, cause) in assumes {
            let got = match catch(f) {
                Ok(()) => "OK\t-".to_string(),
                Err(p) => {
                    let mut lines = p.lines();
                    let head = java_error(lines.next().unwrap());
                    let c = lines
                        .next()
                        .map(|l| java_error(l.trim_start_matches("Caused by: ")))
                        .unwrap_or_else(|| "-".into());
                    assert_eq!(c, cause, "{call} cause");
                    format!("{head}\t{c}")
                }
            };
            log.push((format!("{config}\tassume\t{call}"), got));
        }
        log.push((
            format!("{config}\tprefix\t{}", SysGlobals::current_prefix()),
            format!(
                "{}\t{}\t{}",
                SysGlobals::prefix_property("nightly"),
                SysGlobals::sysprop_random_seed(),
                SysGlobals::sysprop_testfilter()
            ),
        ));
    }
}

#[test]
fn helpers_equal_randomized_test() {
    let expected: BTreeMap<String, String> = rows("helpers.tsv")
        .into_iter()
        .map(|r| (r[..3].join("\t"), r[3..].join("\t")))
        .collect();
    let configs: [(&str, Option<&str>, Option<&str>); 5] = [
        ("default", None, None),
        ("nightly-x2.5", Some("true"), Some("2.5")),
        ("x-1.5", Some("false"), Some("-1.5")),
        ("xNaN", Some("off"), Some("NaN")),
        ("nightly-malformed", Some("maybe"), Some("0")),
    ];
    let log: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    for (config, nightly, multiplier) in configs {
        match nightly {
            Some(v) => system::set_property("tests.nightly", v),
            None => system::clear_property("tests.nightly"),
        };
        match multiplier {
            Some(v) => system::set_property(T::SYSPROP_MULTIPLIER, v),
            None => system::clear_property(T::SYSPROP_MULTIPLIER),
        };
        // One suite per configuration: the group state is read once per
        // context, as per runner in Java.
        let group = ThreadGroup::new(ThreadGroup::main(), "TGRP-HelpersSuite");
        let l = log.clone();
        let config = config.to_string();
        let t = Thread::with_group(&group, "SUITE", move || {
            let runner = Randomness::new(0x1234_ABCD, default_supplier(), vec![]);
            let me = Thread::current();
            let ctx = RandomizedContext::create(
                &me.thread_group().unwrap(),
                "com.carrotsearch.randomizedtesting.RandomizedTestingOracle$HelpersSuite",
                runner.clone(),
                GroupEvaluator::new(),
            );
            ctx.push(runner.clone_for(&me));
            let mut local = Vec::new();
            run_config(&config, &mut local);
            l.lock().unwrap().extend(local);
            ctx.pop_and_destroy();
            ctx.dispose();
        });
        t.start();
        if let Err(p) = t.join() {
            std::panic::resume_unwind(p);
        }
    }
    system::clear_property("tests.nightly");
    system::clear_property(T::SYSPROP_MULTIPLIER);
    let actual: BTreeMap<String, String> = log.lock().unwrap().iter().cloned().collect();
    assert_same_lines("helpers", &expected, &actual);
}

#[test]
fn temp_paths_close_with_their_scope() {
    let group = ThreadGroup::new(ThreadGroup::main(), "TGRP-TempSuite");
    let t = Thread::with_group(&group, "SUITE", || {
        let runner = Randomness::new(1, default_supplier(), vec![]);
        let me = Thread::current();
        let ctx = RandomizedContext::create(
            &me.thread_group().unwrap(),
            "TempSuite",
            runner.clone(),
            GroupEvaluator::new(),
        );
        ctx.push(runner.clone_for(&me));
        let dir = T::new_temp_dir().unwrap();
        std::fs::write(dir.join("f"), b"x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let file = T::new_temp_file_in(crate::LifecycleScope::Suite).unwrap();
        assert!(dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(" has-space"));
        let socket = T::new_server_socket(crate::LifecycleScope::Test).unwrap();
        assert!(socket.local_port() > 0 && !socket.is_closed());
        let close_all = |scope| {
            ctx.close_resources(
                |info: crate::closeable_resource_info::CloseableResourceInfo| {
                    info.get_resource().close().unwrap();
                },
                scope,
            )
        };
        close_all(crate::LifecycleScope::Test);
        assert!(!dir.exists() && file.exists() && socket.is_closed());
        close_all(crate::LifecycleScope::Suite);
        assert!(!file.exists());
        let missing = T::rm_dir(&dir).unwrap_err();
        assert_eq!(
            missing.to_string(),
            format!("Not a folder: {}", dir.display())
        );
        ctx.pop_and_destroy();
        ctx.dispose();
    });
    t.start();
    if let Err(p) = t.join() {
        std::panic::resume_unwind(p);
    }
    crate::jdk::runtime::run_shutdown_hooks();
}
