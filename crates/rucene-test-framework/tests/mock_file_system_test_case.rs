//! `MockFileSystemTestCase` run against every mock filesystem, as Lucene's
//! `TestLeakFS`, `TestHandleLimitFS`, `TestWindowsFS`, `TestVirusCheckingFS`,
//! `TestShuffleFS`, `TestExtrasFS`, `TestDisableFsyncFS` and `TestVerboseFS`
//! (lucene/test-framework/src/test) run it through their `wrap`: each suite
//! extends `MockFileSystemTestCase` (`@SuppressFileSystems("*")`, `extends
//! LuceneTestCase`), runs under the `RandomizedRunner` with `LuceneTestCase`'s
//! rule chain, and starts every test from `LuceneTestCase.createTempDir()`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{JUnitCore, Request};
use rucene::store::java_nio_file::{default_file_system, FileSystemProvider, Path};
use rucene::util::InfoStream;
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, HandleLimitFS, LeakFS, MockFileSystemTestCase, ShuffleFS, VerboseFS,
    VirusCheckingFS, WindowsFS,
};

type Wrap = Arc<dyn Fn(&Path) -> Path + Send + Sync>;

struct Case(Wrap);

impl MockFileSystemTestCase for Case {
    fn wrap(&self, path: &Path) -> Path {
        (self.0)(path)
    }
}

struct Null;

impl InfoStream for Null {
    fn java_class_name(&self) -> &'static str {
        "InfoStream$NoOutput"
    }
    fn message(&self, _: &str, _: &str) {}
    fn is_enabled(&self, _: &str) -> bool {
        true
    }
    fn close(&self) {}
}

fn cases() -> Vec<(&'static str, Wrap)> {
    fn via(provider: Arc<dyn FileSystemProvider>) -> Wrap {
        Arc::new(move |p: &Path| p.with_file_system(provider.get_file_system()))
    }
    vec![
        (
            "TestLeakFS",
            Arc::new(|p: &Path| {
                p.with_file_system(LeakFS::new(default_file_system()).get_file_system())
            }),
        ),
        (
            "TestHandleLimitFS",
            Arc::new(|p: &Path| {
                p.with_file_system(
                    HandleLimitFS::new(default_file_system(), 4096).get_file_system(),
                )
            }),
        ),
        (
            "TestWindowsFS",
            Arc::new(|p: &Path| {
                let fs = WindowsFS::new(default_file_system()).get_file_system();
                Arc::clone(&fs).get_path(&p.to_string(), &[])
            }),
        ),
        (
            "TestVirusCheckingFS",
            Arc::new(|p: &Path| {
                p.with_file_system(
                    VirusCheckingFS::new(default_file_system(), 42).get_file_system(),
                )
            }),
        ),
        (
            "TestShuffleFS",
            via(ShuffleFS::new(default_file_system(), 7)),
        ),
        (
            "TestExtrasFS",
            Arc::new(|p: &Path| {
                p.with_file_system(
                    ExtrasFS::new(default_file_system(), true, false).get_file_system(),
                )
            }),
        ),
        (
            "TestDisableFsyncFS",
            Arc::new(|p: &Path| {
                p.with_file_system(DisableFsyncFS::new(default_file_system()).get_file_system())
            }),
        ),
        (
            "TestVerboseFS",
            Arc::new(|p: &Path| {
                p.with_file_system(
                    VerboseFS::new(default_file_system(), Arc::new(Null)).get_file_system(),
                )
            }),
        ),
    ]
}

#[test]
fn mock_file_system_test_case_passes_for_every_filesystem() {
    type Body = fn(&Case);
    let tests: [(&str, Body); 6] = [
        ("testHashCodeEquals", |c| c.test_hash_code_equals()),
        ("testURI", |c| c.test_uri()),
        ("testURIumlaute", |c| c.test_uri_umlaute()),
        ("testURIchinese", |c| c.test_uri_chinese()),
        ("testDirectoryStreamFiltered", |c| {
            c.test_directory_stream_filtered()
        }),
        ("testDirectoryStreamGlobFiltered", |c| {
            c.test_directory_stream_glob_filtered()
        }),
    ];
    for (name, wrap) in cases() {
        // `Test<FS> extends MockFileSystemTestCase`, which is
        // `@SuppressFileSystems("*")` and `extends LuceneTestCase`.
        let mut builder = Class::builder(format!("org.apache.lucene.tests.mockfile.{name}"))
            .extends(&rucene_test_framework::mockfile::mock_file_system_test_case::class())
            .constructor(Constructor::no_args(|| ()));
        for (method, body) in tests {
            let wrap = Arc::clone(&wrap);
            builder = builder
                .method(Method::new(method).body_any(move || body(&Case(Arc::clone(&wrap)))));
        }
        let class = builder.build();
        let result = JUnitCore::new().run(&Request::a_class(&class));
        if let Some(f) = result.failures().first() {
            let t: &Throwable = f.exception();
            panic!("{name}: {} {t}", f.description());
        }
        assert_eq!(result.run_count(), 6, "{name}");
        // The suite's temporary folder was removed after the suite.
        let prefix = format!("lucene.tests.mockfile.{name}_");
        let left: Vec<String> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&prefix))
            .collect();
        assert!(left.is_empty(), "{name}: left {left:?}");
    }
}
