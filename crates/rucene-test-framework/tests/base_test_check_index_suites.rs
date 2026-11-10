//! The `BaseTestCheckIndex` suite (rmp #355): Lucene's subclass binds the
//! four methods of `BaseTestCheckIndex` to a directory of its own, under the
//! ported runner and `LuceneTestCase`'s rule chain:
//!
//! * `org.apache.lucene.index.TestCheckIndex` (`lucene/core/src/test`):
//!   `setUp()` opens `directory = newDirectory()` after `super.setUp()`,
//!   `tearDown()` closes it before `super.tearDown()`, and `testDeletedDocs`,
//!   `testChecksumsOnly`, `testChecksumsOnlyVerbose` and `testObtainsLock`
//!   call the inherited methods of the same name on it. `BaseTestCheckIndex`
//!   declares no JUnit test, so these four wrappers are the binding.
//!
//! `TestCheckIndex` itself, with these wrappers and its own test methods
//! (`testCheckIndexAllValid`, `testInvalidThreadCountArgument`,
//! `testPriorBrokenCommitPoint`), is bound once, in the core-test port
//! (`core_index/test_check_index.rs`); this file keeps the seeded-draw oracle
//! over the same members.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::error::Result;
use rucene::store::Directory;
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::base_test_check_index;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

mod suite_draws;

/// The instance of `TestCheckIndex`: its `directory` field.
#[derive(Default)]
struct CheckIndexInstance {
    directory: Option<TestDirectory>,
}

impl CheckIndexInstance {
    /// `directory`, as the inherited methods take it.
    fn directory(&self) -> Arc<dyn Directory> {
        self.directory
            .as_ref()
            .expect("setUp opened the directory")
            .as_directory()
    }
}

/// A wrapper test: `public void <name>() throws IOException { <name>(directory); }`.
fn wrapper(name: &str, body: fn(Arc<dyn Directory>) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut CheckIndexInstance| {
            body(inst.directory()).unwrap_or_else(|e| rethrow(e))
        })
}

/// `setUp()`: `super.setUp(); directory = newDirectory();`.
fn set_up(inst: &mut CheckIndexInstance) {
    LuceneTestCase::super_set_up();
    inst.directory = Some(LuceneTestCase::new_directory());
}

/// `tearDown()`: `directory.close(); super.tearDown();`.
fn tear_down(inst: &mut CheckIndexInstance) {
    if let Some(mut directory) = inst.directory.take() {
        directory.close().unwrap_or_else(|e| rethrow(e));
    }
    LuceneTestCase::super_tear_down();
}

/// The members `TestCheckIndex` and the draws class share.
fn with_members(
    builder: randomizedtesting::jdk::reflect::ClassBuilder,
) -> randomizedtesting::jdk::reflect::ClassBuilder {
    builder
        .constructor(Constructor::no_args(CheckIndexInstance::default))
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body(set_up),
        )
        .method(wrapper(
            "testDeletedDocs",
            base_test_check_index::test_deleted_docs,
        ))
        .method(wrapper(
            "testChecksumsOnly",
            base_test_check_index::test_checksums_only,
        ))
        .method(wrapper(
            "testChecksumsOnlyVerbose",
            base_test_check_index::test_checksums_only_verbose,
        ))
        .method(wrapper(
            "testObtainsLock",
            base_test_check_index::test_obtains_lock,
        ))
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestCheckIndexSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestCheckIndexSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestCheckIndexSuiteDraws` of `BaseFormatSuitesOracle`: the members of
/// `TestCheckIndex` that bind the suite, recording the next
/// `random().nextLong()` after each test (in `tearDown`, before the rest of
/// it) and after the class.
fn draws_class() -> Arc<Class> {
    with_members(
        Class::builder("org.apache.lucene.tests.index.TestCheckIndexSuiteDraws")
            .extends(&base_test_check_index::class()),
    )
    .method(
        Method::new("tearDown")
            .throws(&["java.lang.Exception"])
            .body(|inst: &mut CheckIndexInstance| {
                DRAWS.lock().unwrap().push(format!(
                    "{}\tnext\t{}",
                    LuceneTestCase::get_test_name().unwrap_or_default(),
                    LuceneTestCase::random().next_long()
                ));
                tear_down(inst);
            }),
    )
    .method(
        Method::new("recordClassDraw")
            .modifiers(modifier::PUBLIC | modifier::STATIC)
            .annotate(randomizedtesting::junit::AfterClass)
            .static_body(|| {
                DRAWS.lock().unwrap().push(format!(
                    "#class\tnext\t{}",
                    LuceneTestCase::random().next_long()
                ));
            }),
    )
    .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #355): every row of
/// `base_check_index_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_check_index_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_check_index_draws.tsv", draws_class, &DRAWS);
}
