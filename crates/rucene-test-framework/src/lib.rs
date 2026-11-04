//! Port of Apache Lucene 10.5.0 `lucene/test-framework`.
//!
//! The Lucene module (`org.apache.lucene.tests.*`) is the base of Lucene's own
//! tests: `LuceneTestCase`, the asserting, mock and fault-injecting wrappers
//! and the `Base…TestCase` suites. It runs on `randomizedtesting`
//! (`com.carrotsearch.randomizedtesting` 2.8.1) and depends on `lucene/core`
//! (`rucene`) and `lucene/codecs` (`rucene-codecs`), as here.
//!
//! Ported so far: [`mockfile`] (the fault-injecting filesystems), the static
//! environment and randomness API of [`util::LuceneTestCase`], and the members
//! of other classes they call. The rest of the module is ported by later
//! tasks.

pub mod hamcrest;
pub mod jdk;
pub mod mockfile;
pub mod store;
pub mod util;
