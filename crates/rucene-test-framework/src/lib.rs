//! Port of Apache Lucene 10.5.0 `lucene/test-framework`.
//!
//! The Lucene module (`org.apache.lucene.tests.*`) is the base of Lucene's own
//! tests: `LuceneTestCase`, the asserting, mock and fault-injecting wrappers
//! and the `Base…TestCase` suites. It runs on `randomizedtesting`
//! (`com.carrotsearch.randomizedtesting` 2.8.1) and depends on `lucene/core`
//! (`rucene`) and `lucene/codecs` (`rucene-codecs`), as here. Its content is
//! ported by later tasks.
