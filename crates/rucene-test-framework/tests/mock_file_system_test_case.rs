//! `MockFileSystemTestCase` run against every mock filesystem, as Lucene's
//! `TestLeakFS`, `TestHandleLimitFS`, `TestWindowsFS`, `TestVirusCheckingFS`,
//! `TestShuffleFS`, `TestExtrasFS`, `TestDisableFsyncFS` and `TestVerboseFS`
//! (lucene/test-framework/src/test) run it through their `wrap`.

mod oracle_support;

use std::sync::{Arc, Mutex};

use oracle_support::TempDir;
use rucene::store::java_nio_file::{default_file_system, FileSystemProvider, Path};
use rucene::util::InfoStream;
use rucene_test_framework::mockfile::{
    DisableFsyncFS, ExtrasFS, HandleLimitFS, LeakFS, MockFileSystemTestCase, ShuffleFS, VerboseFS,
    VirusCheckingFS, WindowsFS,
};

struct Case {
    wrap: Box<dyn Fn(&Path) -> Path>,
    temps: Mutex<Vec<TempDir>>,
}

impl MockFileSystemTestCase for Case {
    fn wrap(&self, path: &Path) -> Path {
        (self.wrap)(path)
    }

    fn create_temp_dir(&self) -> Path {
        let t = TempDir::new("mockfs");
        let p = Path::new(default_file_system(), t.0.as_os_str());
        self.temps.lock().unwrap().push(t);
        p
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

type Wrap = Box<dyn Fn(&Path) -> Path>;

fn cases() -> Vec<(&'static str, Wrap)> {
    fn via(provider: Arc<dyn FileSystemProvider>) -> Wrap {
        Box::new(move |p: &Path| p.with_file_system(provider.get_file_system()))
    }
    vec![
        (
            "LeakFS",
            Box::new(|p: &Path| {
                p.with_file_system(LeakFS::new(default_file_system()).get_file_system())
            }),
        ),
        (
            "HandleLimitFS",
            Box::new(|p: &Path| {
                p.with_file_system(
                    HandleLimitFS::new(default_file_system(), 4096).get_file_system(),
                )
            }),
        ),
        (
            "WindowsFS",
            Box::new(|p: &Path| {
                let fs = WindowsFS::new(default_file_system()).get_file_system();
                Arc::clone(&fs).get_path(&p.to_string(), &[])
            }),
        ),
        (
            "VirusCheckingFS",
            via(VirusCheckingFS::new(default_file_system(), 42)),
        ),
        ("ShuffleFS", via(ShuffleFS::new(default_file_system(), 7))),
        (
            "ExtrasFS",
            via(ExtrasFS::new(default_file_system(), true, false)),
        ),
        (
            "DisableFsyncFS",
            via(DisableFsyncFS::new(default_file_system())),
        ),
        (
            "VerboseFS",
            via(VerboseFS::new(default_file_system(), Arc::new(Null))),
        ),
    ]
}

#[test]
fn mock_file_system_test_case_passes_for_every_filesystem() {
    for (name, wrap) in cases() {
        let case = Case {
            wrap,
            temps: Mutex::new(Vec::new()),
        };
        let run = |what: &str, f: &dyn Fn(&Case)| {
            if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&case))) {
                let t = randomizedtesting::jdk::exception::Throwable::from_panic(p);
                panic!("{name}.{what}: {t}");
            }
        };
        run("testHashCodeEquals", &|c| c.test_hash_code_equals());
        run("testURI", &|c| c.test_uri());
        run("testURIumlaute", &|c| c.test_uri_umlaute());
        run("testURIchinese", &|c| c.test_uri_chinese());
        run("testDirectoryStreamFiltered", &|c| {
            c.test_directory_stream_filtered()
        });
        run("testDirectoryStreamGlobFiltered", &|c| {
            c.test_directory_stream_glob_filtered()
        });
    }
}
