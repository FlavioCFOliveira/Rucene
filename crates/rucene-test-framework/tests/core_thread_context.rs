//! Threads that `rucene` starts take their creator's thread group, as the
//! JVM gives every thread `lucene-core` starts, so that test code running on
//! them (a codec's `FieldsConsumer` on a merge thread, for example) finds the
//! suite's `RandomizedContext` through `LuceneTestCase.random()`.
//!
//! Regression test for the hang of `TestDirectPostingsFormat` on the master
//! seed `9D704B962994A060`: `BasePostingsFormatTestCase.testInvertedWrite`'s
//! `FieldsConsumer.write` calls `random()` on the `ConcurrentMergeScheduler`
//! merge thread, where the context lookup threw, the merge thread died
//! without finishing its merge, and `forceMerge` waited for ever. Each check
//! below runs the call under `catch_unwind` on the started thread, so a
//! missing context fails the test instead of hanging it.

#[path = "suite_seeds/mod.rs"]
mod suite_seeds;

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::store::ByteBuffersDirectory;
use rucene::util::{InfoStream, NamedThreadFactory};
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// What a started thread observed: its group's name and whether
/// `LuceneTestCase.random()` returned.
#[derive(Debug, Clone, PartialEq)]
struct Observed {
    group: Option<String>,
    random: Result<(), String>,
}

/// Runs on the calling thread: the group of its `Thread` handle and the
/// outcome of one `random()` draw.
fn observe() -> Observed {
    let group = Thread::current()
        .thread_group()
        .map(|g| g.name().to_string());
    let random = std::panic::catch_unwind(|| {
        let _ = LuceneTestCase::random().next_int();
    })
    .map_err(|payload| {
        randomizedtesting::jdk::exception::Throwable::from_panic(payload)
            .message()
            .unwrap_or("null")
            .to_string()
    });
    Observed { group, random }
}

/// The calling (test) thread's group name.
fn creator_group() -> Option<String> {
    Thread::current()
        .thread_group()
        .map(|g| g.name().to_string())
}

/// Asserts that every observation was made in `group` and drew a random.
fn assert_inherited(group: &Option<String>, observed: &[Observed]) {
    Assert::assert_true(!observed.is_empty());
    for o in observed {
        Assert::assert_equals_str(None, group.as_deref(), o.group.as_deref());
        if let Err(message) = &o.random {
            Assert::fail(&format!("random() on a started thread threw: {message}"));
        }
    }
}

/// An `InfoStream` that observes the merge threads that write to it.
struct MergeThreadObserver(Mutex<Vec<Observed>>);

impl InfoStream for MergeThreadObserver {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.MergeThreadObserver"
    }
    fn message(&self, _component: &str, _message: &str) {
        let on_merge_thread = std::thread::current()
            .name()
            .is_some_and(|n| n.starts_with("Lucene Merge Thread"));
        if on_merge_thread {
            self.0.lock().unwrap().push(observe());
        }
    }
    fn is_enabled(&self, _component: &str) -> bool {
        true
    }
    fn close(&self) {}
}

/// A merge thread of `ConcurrentMergeScheduler` is in the group of the
/// thread that started the merge and draws from the suite's context.
fn test_merge_thread_inherits_context() {
    let observer = Arc::new(MergeThreadObserver(Mutex::new(Vec::new())));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(ConcurrentMergeScheduler::new()))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&observer) as Arc<dyn InfoStream>)
        .unwrap();
    let w = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for id in ["0", "1"] {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", id.to_string(), Store::NO).unwrap(),
        ));
        w.add_document(&doc).unwrap();
        w.commit().unwrap();
    }
    w.force_merge(1).unwrap();
    w.close().unwrap();
    let observed = observer.0.lock().unwrap().clone();
    assert_inherited(&creator_group(), &observed);
}

/// A thread of `NamedThreadFactory` is in its creator's group and draws from
/// the suite's context.
fn test_named_thread_inherits_context() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    NamedThreadFactory::new("test")
        .new_thread(move || sink.lock().unwrap().push(observe()))
        .unwrap()
        .join()
        .unwrap();
    let observed = observed.lock().unwrap().clone();
    assert_inherited(&creator_group(), &observed);
}

/// A thread started outside `rucene` (a bare `std::thread::spawn`) is still
/// adopted in the `main` group, which has no context: the documented
/// divergence of `randomizedtesting::jdk::thread` that remains.
fn test_foreign_thread_has_no_context() {
    let observed = std::thread::spawn(observe).join().unwrap();
    Assert::assert_equals_str(None, Some("main"), observed.group.as_deref());
    Assert::assert_true(observed.random.is_err());
}

fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.util.TestCoreThreadContext")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("testMergeThreadInheritsContext")
                .body_any(test_merge_thread_inherits_context),
        )
        .method(
            Method::new("testNamedThreadInheritsContext")
                .body_any(test_named_thread_inherits_context),
        )
        .method(
            Method::new("testForeignThreadHasNoContext")
                .body_any(test_foreign_thread_has_no_context),
        )
        .build()
}

#[test]
fn core_threads_inherit_the_randomized_context() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_seeds::run_suite(class);
}
