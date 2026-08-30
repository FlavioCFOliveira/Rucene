//! Internal bridges to package-private internals, for use by the Lucene test
//! framework only.
//!
//! Port of `org.apache.lucene.internal.tests`. Despite the name, this is
//! production code in Lucene Core: the accessors are declared here and
//! registered by the very classes whose internals they expose, so that the
//! separate test-framework module can reach them without widening their
//! visibility. [`TestSecrets`] is the registry.
//!
//! # Module renamed to `tests_hooks`
//!
//! Lucene's package is called `tests`. A Rust module of that name inside `src/`
//! would collide with two established conventions of this crate: the
//! `#[cfg(test)] mod tests` block that every module carries, and the top-level
//! `tests/` directory that holds the integration and portability suites.
//! `tests_hooks` keeps the meaning — the hooks the test framework uses — while
//! leaving both conventions intact. The type names are unchanged.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`TestSecrets`] | `TestSecrets` |
//! | [`IndexPackageAccess`], [`FieldInfosBuilder`] | `IndexPackageAccess`, `IndexPackageAccess.FieldInfosBuilder` |
//! | [`IndexWriterAccess`] | `IndexWriterAccess` |
//! | [`SegmentReaderAccess`] | `SegmentReaderAccess` |
//! | [`ConcurrentMergeSchedulerAccess`] | `ConcurrentMergeSchedulerAccess` |
//! | [`FilterIndexInputAccess`] | `FilterIndexInputAccess` |
//!
//! # Divergences from Lucene 10.5.0
//!
//! The accessors are traits rather than interfaces backed by anonymous classes,
//! and [`TestSecrets`] enforces the set-once contract but not the caller
//! restriction, which needs a stack walker Rust does not have. Each item states
//! its own divergences; see [`TestSecrets`] for the registry-level ones.
//!
//! No implementation of these traits is registered yet: in Lucene each one is
//! installed by the static initializer of `IndexWriter`, `SegmentReader`,
//! `ConcurrentMergeScheduler` and `FilterIndexInput`, which have no equivalent
//! initialization point in this port, so registration will be added with the
//! test framework itself.

#![deny(unsafe_code)]

pub mod concurrent_merge_scheduler_access;
pub mod filter_index_input_access;
pub mod index_package_access;
pub mod index_writer_access;
pub mod segment_reader_access;
pub mod test_secrets;

pub use concurrent_merge_scheduler_access::ConcurrentMergeSchedulerAccess;
pub use filter_index_input_access::FilterIndexInputAccess;
pub use index_package_access::{FieldInfosBuilder, IndexPackageAccess};
pub use index_writer_access::IndexWriterAccess;
pub use segment_reader_access::SegmentReaderAccess;
pub use test_secrets::TestSecrets;
