//! The bindings of `SearchEquivalenceTestBase` and `MatchesTestBase`
//! (rmp #356), as Lucene's own subclasses declare them
//! (`lucene/core/src/test/org/apache/lucene/search/`). The subclasses of
//! `BaseExplanationTestCase` (`TestSimpleExplanations`,
//! `TestComplexExplanations`) are bound once, with their test methods, in
//! `core_search_explanations_tests.rs`, and `TestMatchesIterator` (over
//! `MatchesTestBase`) in `core_search_tests.rs`:
//!
//! * `TestSloppyPhraseQuery2`, `TestSimpleSearchEquivalence` over
//!   `SearchEquivalenceTestBase` (`TestApproximationSearchEquivalence` is
//!   bound once, with its test methods, in `core_search_tests.rs`);
//!
//! These base classes declare no test method: they are helpers and class
//! fixtures (`@BeforeClass`/`@AfterClass`, `setUp()`/`tearDown()`) for the
//! subclasses' own test methods, which belong to the core-test port
//! (sprint 7). Each binding therefore runs no inherited test; its suite run
//! exercises the class fixtures, as Lucene's runner does for a class with
//! no test of its own.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene_test_framework::search::search_equivalence_test_base::{self, EquivalenceTestInstance};

mod suite_seeds;
use suite_seeds::run_suite;

fn equivalence_class(name: &str) -> Arc<Class> {
    Class::builder(name)
        .extends(&search_equivalence_test_base::class())
        .constructor(Constructor::no_args(EquivalenceTestInstance::default))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

#[test]
fn test_sloppy_phrase_query2() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| equivalence_class("org.apache.lucene.search.TestSloppyPhraseQuery2"));
}

#[test]
fn test_simple_search_equivalence() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(|| equivalence_class("org.apache.lucene.search.TestSimpleSearchEquivalence"));
}
