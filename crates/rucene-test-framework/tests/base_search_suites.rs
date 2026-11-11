//! The bindings of `SearchEquivalenceTestBase` and `MatchesTestBase`
//! (rmp #356), as Lucene's own subclasses declare them
//! (`lucene/core/src/test/org/apache/lucene/search/`).
//!
//! Every subclass is now bound once, with its own test methods, in its core
//! test target, so this file binds none:
//!
//! * the subclasses of `BaseExplanationTestCase` (`TestSimpleExplanations`,
//!   `TestComplexExplanations`) in `core_search_explanations_tests.rs`;
//! * `TestMatchesIterator` (over `MatchesTestBase`) in
//!   `core_search_tests.rs`;
//! * `TestSloppyPhraseQuery2`, `TestSimpleSearchEquivalence` and
//!   `TestApproximationSearchEquivalence` (over `SearchEquivalenceTestBase`)
//!   in `core_search_tests.rs`.
