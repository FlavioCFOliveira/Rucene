//! Port of Apache Lucene 10.5.0 `lucene/test-framework`.
//!
//! The Lucene module (`org.apache.lucene.tests.*`) is the base of Lucene's own
//! tests: `LuceneTestCase`, the asserting, mock and fault-injecting wrappers
//! and the `Base…TestCase` suites. It runs on `randomizedtesting`
//! (`com.carrotsearch.randomizedtesting` 2.8.1) and depends on `lucene/core`
//! (`rucene`) and `lucene/codecs` (`rucene-codecs`), as here.
//!
//! Ported so far: [`mockfile`] (the fault-injecting filesystems), the static
//! environment and randomness API of [`util::LuceneTestCase`], the test
//! codecs of [`codecs`] (`asserting`, `cheapbastard`, `compressing`,
//! `cranky`, `vector`), the helpers of [`util`] (`TestUtil`, `LineFileDocs`,
//! `automaton::AutomatonTestUtil`, `fst::FSTTester`, `CloseableDirectory`,
//! ...), and the members of other classes they call. The rest of the module
//! is ported by later tasks.

/// Java's `assert condition [: message];` under `-ea` in a method that
/// returns [`rucene::error::Result`]: a failed assertion returns
/// `Err(LuceneError::AssertionError(message))` from the enclosing function,
/// the error a Java `AssertionError` stands for in this port (the message is
/// empty for an `assert` without one, Java's `null`).
///
/// **Divergence.** Java evaluates `assert` only with `-ea` (as Lucene's
/// builds run tests); the port evaluates it when `debug_assertions` are on.
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::AssertionError(String::new()));
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::AssertionError(format!($($msg)+)));
        }
    };
}

/// Java's `assert condition [: message];` under `-ea` in a method whose Rust
/// signature cannot fail (Java's `docID()`, `cost()`, `Bits.get(int)`, ...):
/// a failed assertion throws `java.lang.AssertionError`, that is panics with
/// its `Throwable.toString()` ([`randomizedtesting::jdk::exception`]), as the
/// unchecked Java error unwinds to the test.
macro_rules! java_assert_unchecked {
    ($cond:expr) => {
        if cfg!(debug_assertions) {
            randomizedtesting::jdk::exception::java_assert_no_message($cond)
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) {
            randomizedtesting::jdk::exception::java_assert($cond, || format!($($msg)+))
        }
    };
}

pub mod codecs;
pub mod hamcrest;
pub mod index;
pub mod jdk;
pub mod mockfile;
pub mod search;
pub mod store;
pub mod util;

mod spi;

pub use spi::class_loader;
