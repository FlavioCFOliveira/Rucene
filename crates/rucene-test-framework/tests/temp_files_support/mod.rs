//! The suites of `TestTempFilesSuite.java` (the oracle of `LuceneTestCase`'s
//! temporary-files rule), declared for the ported runner.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Marker, Method};
use randomizedtesting::junit::Assert;
use rucene::store::java_nio_file::{FileSystemProvider, Files, Path};
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, FilterFileSystemProvider, HandleLimitFS, LeakFS, MaxOpenHandles,
    ShuffleFS, VerboseFS, WindowsFS,
};
use rucene_test_framework::util::lucene_test_case::{
    LuceneTestCase, SuppressFileSystems, SUPPRESS_FSYNC,
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
    // Each suite `extends LuceneTestCase` (the nested classes are `static`).
    let mut b = Class::builder(name).extends(&LuceneTestCase::class());
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
    b.constructor(Constructor::no_args(|| ()))
        .method(Method::new("testRecord").body_any(move || {
            ROWS.lock().unwrap().push(record());
            if failing {
                Assert::fail("boom");
            }
        }))
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
