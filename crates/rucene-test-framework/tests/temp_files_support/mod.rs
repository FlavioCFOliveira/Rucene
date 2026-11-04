//! The suites of `TestTempFilesSuite.java` (the oracle of `LuceneTestCase`'s
//! temporary-files rule), declared for the ported runner.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use randomizedtesting::annotations::{SeedDecoratorClass, SeedDecorators};
use randomizedtesting::jdk::reflect::{
    Class, Constructor, Field, JavaAnnotation, Marker, Method, Value,
};
use randomizedtesting::junit::{Assert, ClassRule, Rule, Test, TestRule};
use randomizedtesting::rules::TestRuleAdapterRule;
use rucene::store::java_nio_file::{FileSystemProvider, Files, Path};
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, FilterFileSystemProvider, HandleLimitFS, LeakFS, MaxOpenHandles,
    ShuffleFS, VerboseFS, WindowsFS,
};
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase, SuppressFileSystems, SUITE_FAILURE_MARKER, SUPPRESS_FSYNC,
    TEMP_FILES_CLEANUP_RULE,
};
use rucene_test_framework::util::test_rule_mark_failure::{
    TestRuleMarkFailure, TestRuleMarkFailureRule,
};

pub static ROWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The oracle's `describe(provider)`: the chain of filter providers, with
/// the parameters the random choices set.
pub fn describe(provider: Arc<dyn FileSystemProvider>) -> String {
    let mut sb = String::new();
    let mut p = provider;
    loop {
        let any = p.as_any();
        let (name, params, delegate): (&str, String, Arc<dyn FileSystemProvider>) =
            if let Some(x) = any.downcast_ref::<ShuffleFS>() {
                (
                    "ShuffleFS",
                    format!("(seed={})", x.seed()),
                    Arc::clone(x.filter().delegate()),
                )
            } else if let Some(x) = any.downcast_ref::<HandleLimitFS>() {
                (
                    "HandleLimitFS",
                    format!("(limit={})", x.limit()),
                    Arc::clone(x.filter().delegate()),
                )
            } else if let Some(x) = any.downcast_ref::<ExtrasFS>() {
                (
                    "ExtrasFS",
                    format!(
                        "(active={},createDirectory={})",
                        x.active(),
                        x.create_directory()
                    ),
                    Arc::clone(x.filter().delegate()),
                )
            } else if let Some(x) = any.downcast_ref::<LeakFS>() {
                ("LeakFS", String::new(), Arc::clone(x.filter().delegate()))
            } else if let Some(x) = any.downcast_ref::<WindowsFS>() {
                (
                    "WindowsFS",
                    String::new(),
                    Arc::clone(x.filter().delegate()),
                )
            } else if let Some(x) = any.downcast_ref::<DisableFsyncFS>() {
                (
                    "DisableFsyncFS",
                    String::new(),
                    Arc::clone(x.filter().delegate()),
                )
            } else if let Some(x) = any.downcast_ref::<VerboseFS>() {
                (
                    "VerboseFS",
                    String::new(),
                    Arc::clone(x.filter().delegate()),
                )
            } else {
                break;
            };
        sb.push_str(name);
        sb.push_str(&params);
        sb.push_str(" > ");
        p = delegate;
    }
    sb.push_str("default");
    sb
}

/// The real path of `java.io.tmpdir`.
pub fn tmp() -> String {
    std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// The oracle's `record()`.
pub fn record() -> String {
    let tmp = tmp();
    let a = LuceneTestCase::create_temp_dir();
    let b = LuceneTestCase::create_temp_dir_prefix("x");
    let c = LuceneTestCase::create_temp_dir();
    let d = LuceneTestCase::create_temp_file().unwrap();
    let e = LuceneTestCase::create_temp_file_with("p", ".s").unwrap();
    let mut sb = describe(a.get_file_system().provider());
    for p in [&a, &b, &c, &d, &e] {
        sb.push('\t');
        sb.push_str(&p.to_string().replace(&tmp, "$T"));
    }
    sb.push_str(&format!(
        "\t{}{}",
        Files::is_directory(&a, &[]),
        Files::is_regular_file(&d, &[])
    ));
    sb
}

/// The suite `simple` (`TestTempFilesSuite` or one of its nested classes).
pub fn suite(simple: &str) -> Arc<Class> {
    let name = if simple == "TestTempFilesSuite" {
        "org.apache.lucene.tests.util.TestTempFilesSuite".to_string()
    } else {
        format!("org.apache.lucene.tests.util.TestTempFilesSuite${simple}")
    };
    // `LuceneTestCase` is `@SeedDecorators({MixWithSuiteName.class})`.
    let mut b = Class::builder(name).annotate(SeedDecorators {
        value: vec![SeedDecoratorClass::mix_with_suite_name()],
    });
    match simple {
        "TestSuppressed" => {
            b = b.annotate(SuppressFileSystems {
                value: vec!["ShuffleFS".into(), "LeakFS".into(), "ExtrasFS".into()],
            })
        }
        "TestFsyncLimit" => {
            b = b
                .annotate(Marker(&SUPPRESS_FSYNC))
                .annotate(MaxOpenHandles { limit: 17 })
        }
        "TestAll" => {
            b = b.annotate(SuppressFileSystems {
                value: vec!["*".into()],
            })
        }
        _ => {}
    }
    let failing = simple == "TestFailing";
    let _ = |a: &dyn JavaAnnotation| a.to_java_string();
    b.constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("suiteFailureMarker", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::new(TestRuleMarkFailureRule(Arc::clone(&SUITE_FAILURE_MARKER)))
                        as Arc<dyn TestRule>,
                )
            })
            .annotate(ClassRule),
        )
        .field(
            Field::new_static("tempFilesCleanupRule", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::new(TestRuleAdapterRule(Arc::clone(&TEMP_FILES_CLEANUP_RULE)))
                        as Arc<dyn TestRule>,
                )
            })
            .annotate(ClassRule),
        )
        // `LuceneTestCase.testRules`' `testFailureMarker`, chained to the
        // suite marker.
        .field(
            Field::new_instance::<()>("testFailureMarker", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(TestRuleMarkFailureRule(Arc::new(
                    TestRuleMarkFailure::new(vec![Arc::clone(&SUITE_FAILURE_MARKER)]),
                ))) as Arc<dyn TestRule>)
            })
            .annotate(Rule),
        )
        .method(
            Method::new("testRecord")
                .annotate(Test::new())
                .body_any(move || {
                    ROWS.lock().unwrap().push(record());
                    if failing {
                        Assert::fail("boom");
                    }
                }),
        )
        .build()
}

/// The entries of `java.io.tmpdir` starting with `prefix`, sorted.
pub fn left(prefix: &str) -> Vec<String> {
    let mut l: Vec<String> = std::fs::read_dir(tmp())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    l.sort();
    l
}

pub fn unused(_: &Path) {}
