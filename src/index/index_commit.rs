//! Commit-point abstractions ported from `org.apache.lucene.index`.
//!
//! This module holds the point-in-time view of an index and the contracts used
//! to commit several such views atomically:
//!
//! - [`IndexCommit`] — equivalent to `org.apache.lucene.index.IndexCommit`;
//! - [`TwoPhaseCommit`] — equivalent to
//!   `org.apache.lucene.index.TwoPhaseCommit`;
//! - [`execute`] and [`TwoPhaseCommitError`] — equivalent to
//!   `org.apache.lucene.index.TwoPhaseCommitTool` and its two nested
//!   exceptions.
//!
//! The concrete `IndexCommit` implementations live next to the component that
//! produces them, mirroring Lucene: `StandardDirectoryReader.ReaderCommit`
//! stays in [`crate::index::directory_reader`], and the snapshot wrapper stays
//! in [`crate::index::index_deletion_policy`].

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::io;
use std::sync::Arc;

use thiserror::Error;

use crate::error::{LuceneError, Result};
use crate::index::directory_reader::StandardDirectoryReader;
use crate::store::Directory;

// -----------------------------------------------------------------------------
// IndexCommit
// -----------------------------------------------------------------------------

/// Returns the identity of a `Directory` instance.
///
/// Java compares directories with reference equality (`==`) and hashes them
/// with the identity hash code. The Rust analogue is the address of the
/// allocation behind the `Arc`, which is exactly what [`Arc::ptr_eq`] compares.
fn directory_identity(directory: &Arc<dyn Directory>) -> usize {
    Arc::as_ptr(directory) as *const () as usize
}

/// Returns `java.lang.Long.hashCode(value)`.
fn java_long_hash_code(value: i64) -> u32 {
    ((value ^ (((value as u64) >> 32) as i64)) as i32) as u32
}

/// Represents a single commit into an index as seen by
/// [`IndexDeletionPolicy`](crate::index::IndexDeletionPolicy) or
/// [`IndexReader`](crate::index::IndexReader).
///
/// Changes to the content of an index become visible only after the writer that
/// made them commits by writing a new segments file (`segments_N`). That point
/// in time is an index commit. Each commit point has a unique segments file
/// associated with it, and a later commit point has a larger `N`.
///
/// Equivalent to `org.apache.lucene.index.IndexCommit`.
///
/// # Ordering and equality
///
/// Java declares `IndexCommit implements Comparable<IndexCommit>` and overrides
/// `equals`/`hashCode`. Because `dyn IndexCommit` cannot usefully implement
/// [`PartialEq`] or [`Ord`] (both would need `Self: Sized` or a total order
/// across unrelated implementations), those three methods are ported as the
/// provided methods [`IndexCommit::commit_equals`],
/// [`IndexCommit::commit_hash`] and [`IndexCommit::compare_to`], with exactly
/// the Java semantics. Implementations must not override them.
pub trait IndexCommit: Send + Sync + Debug {
    /// Returns the segments file (`segments_N`) associated with this commit point.
    ///
    /// Equivalent to `IndexCommit.getSegmentsFileName()`.
    fn get_segments_file_name(&self) -> String;

    /// Returns all index files referenced by this commit point.
    ///
    /// Equivalent to `IndexCommit.getFileNames()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the referenced segments cannot be enumerated.
    fn get_file_names(&self) -> Result<HashSet<String>>;

    /// Returns the directory for the index.
    ///
    /// Equivalent to `IndexCommit.getDirectory()`.
    fn get_directory(&self) -> Arc<dyn Directory>;

    /// Deletes this commit point.
    ///
    /// This only applies when the commit point is used in the context of an
    /// `IndexWriter`'s [`IndexDeletionPolicy`](crate::index::IndexDeletionPolicy):
    /// upon calling this, the writer is notified that this commit point should
    /// be deleted. The decision is taken by the deletion policy in effect, so
    /// this should only be called from its `on_init` or `on_commit` methods.
    ///
    /// Equivalent to `IndexCommit.delete()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] for commit points that do
    /// not support deletion, mirroring the `UnsupportedOperationException`
    /// thrown by the corresponding Java implementations.
    fn delete(&self) -> Result<()>;

    /// Returns `true` if this commit should be deleted.
    ///
    /// Equivalent to `IndexCommit.isDeleted()`.
    fn is_deleted(&self) -> bool;

    /// Returns the number of segments referenced by this commit.
    ///
    /// Equivalent to `IndexCommit.getSegmentCount()`.
    fn get_segment_count(&self) -> i32;

    /// Returns the generation (the `_N` in `segments_N`) for this commit.
    ///
    /// Equivalent to `IndexCommit.getGeneration()`.
    fn get_generation(&self) -> i64;

    /// Returns user data previously attached to this commit.
    ///
    /// Equivalent to `IndexCommit.getUserData()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the user data cannot be read.
    fn get_user_data(&self) -> Result<HashMap<String, String>>;

    /// Returns the reader that produced this NRT commit point, if any.
    ///
    /// Equivalent to the package-private `IndexCommit.getReader()`, which is
    /// used by `IndexWriter` to initialise from a commit pulled from an NRT or
    /// non-NRT reader. The default returns `None`, as in Java.
    fn get_reader(&self) -> Option<Arc<StandardDirectoryReader>> {
        None
    }

    /// Returns `true` if both commits have the same directory *instance* and
    /// the same generation.
    ///
    /// Equivalent to `IndexCommit.equals(Object)`.
    fn commit_equals(&self, other: &dyn IndexCommit) -> bool {
        directory_identity(&self.get_directory()) == directory_identity(&other.get_directory())
            && self.get_generation() == other.get_generation()
    }

    /// Returns a hash consistent with [`IndexCommit::commit_equals`].
    ///
    /// Equivalent to `IndexCommit.hashCode()`, which combines the directory's
    /// identity hash with `Long.hashCode(getGeneration())`. Rucene uses the
    /// directory allocation address in place of the JVM identity hash.
    fn commit_hash(&self) -> u64 {
        (directory_identity(&self.get_directory()) as u64)
            .wrapping_add(u64::from(java_long_hash_code(self.get_generation())))
    }

    /// Compares two commit points of the same directory by generation.
    ///
    /// Equivalent to `IndexCommit.compareTo(IndexCommit)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the two commits come
    /// from different `Directory` instances, mirroring the
    /// `UnsupportedOperationException` thrown by Java.
    fn compare_to(&self, other: &dyn IndexCommit) -> Result<Ordering> {
        if directory_identity(&self.get_directory()) != directory_identity(&other.get_directory()) {
            return Err(LuceneError::UnsupportedOperation(
                "cannot compare IndexCommits from different Directory instances".to_string(),
            ));
        }
        Ok(self.get_generation().cmp(&other.get_generation()))
    }
}

// -----------------------------------------------------------------------------
// TwoPhaseCommit
// -----------------------------------------------------------------------------

/// Contract for implementations that support a 2-phase commit.
///
/// Use [`execute`] to run a 2-phase commit algorithm over several
/// `TwoPhaseCommit` objects.
///
/// Equivalent to `org.apache.lucene.index.TwoPhaseCommit`.
///
/// # Interior mutability
///
/// Java's `TwoPhaseCommit` is implemented by `IndexWriter`, whose methods are
/// internally synchronised. Rucene therefore takes `&self` — matching the
/// [`IndexWriter`](crate::index::DirectoryReaderIndexWriter) trait already used
/// by [`crate::index::directory_reader`] — and leaves the synchronisation to
/// the implementation.
pub trait TwoPhaseCommit: Debug {
    /// Runs the first stage of a 2-phase commit.
    ///
    /// Implementations should do as much work as possible here but avoid
    /// actually committing changes. If the 2-phase commit fails,
    /// [`TwoPhaseCommit::rollback`] is called to discard all changes made since
    /// the last successful commit.
    ///
    /// Equivalent to `TwoPhaseCommit.prepareCommit()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the preparation fails.
    fn prepare_commit(&self) -> Result<i64>;

    /// Runs the second phase of a 2-phase commit.
    ///
    /// Implementations should ideally do very little work here; once it
    /// returns, the caller may assume the changes were successfully committed
    /// to the underlying storage.
    ///
    /// Equivalent to `TwoPhaseCommit.commit()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit fails.
    fn commit(&self) -> Result<i64>;

    /// Discards any changes that occurred since the last commit.
    ///
    /// Equivalent to `TwoPhaseCommit.rollback()`.
    ///
    /// # Errors
    ///
    /// Returns an error if the rollback fails.
    fn rollback(&self) -> Result<()>;
}

/// Failure raised by [`execute`].
///
/// Equivalent to the two nested exceptions of
/// `org.apache.lucene.index.TwoPhaseCommitTool`:
/// `PrepareCommitFailException` and `CommitFailException`. Both extend
/// `IOException` in Java, which is why [`LuceneError::Io`] is the target of the
/// [`From`] conversion below.
#[derive(Debug, Error)]
pub enum TwoPhaseCommitError {
    /// An object failed to [`TwoPhaseCommit::prepare_commit`].
    ///
    /// Equivalent to `TwoPhaseCommitTool.PrepareCommitFailException`.
    #[error("prepareCommit() failed on {object}")]
    PrepareCommitFail {
        /// Debug rendering of the object that failed, mirroring the Java
        /// message, which appends `obj.toString()`.
        object: String,
        /// The underlying failure.
        #[source]
        cause: Box<LuceneError>,
    },

    /// An object failed to [`TwoPhaseCommit::commit`].
    ///
    /// Equivalent to `TwoPhaseCommitTool.CommitFailException`.
    #[error("commit() failed on {object}")]
    CommitFail {
        /// Debug rendering of the object that failed, mirroring the Java
        /// message, which appends `obj.toString()`.
        object: String,
        /// The underlying failure.
        #[source]
        cause: Box<LuceneError>,
    },
}

impl TwoPhaseCommitError {
    /// Returns the debug rendering of the object that failed.
    pub fn object(&self) -> &str {
        match self {
            Self::PrepareCommitFail { object, .. } | Self::CommitFail { object, .. } => object,
        }
    }

    /// Returns the underlying failure.
    pub fn cause(&self) -> &LuceneError {
        match self {
            Self::PrepareCommitFail { cause, .. } | Self::CommitFail { cause, .. } => cause,
        }
    }
}

impl From<TwoPhaseCommitError> for LuceneError {
    fn from(error: TwoPhaseCommitError) -> Self {
        // Both Java exceptions extend IOException; wrapping preserves the whole
        // cause chain through `std::error::Error::source`.
        LuceneError::Io(io::Error::other(error))
    }
}

/// Rolls back all objects, discarding any error that occurs.
///
/// Equivalent to the private `TwoPhaseCommitTool.rollback(TwoPhaseCommit...)`.
fn rollback_all(objects: &[Option<&dyn TwoPhaseCommit>]) {
    for tpc in objects.iter().flatten() {
        // Ignore any failure during rollback: every object must be rolled back.
        let _ = tpc.rollback();
    }
}

/// Executes a 2-phase commit algorithm over `objects`.
///
/// All objects are first asked to [`TwoPhaseCommit::prepare_commit`]; only if
/// every one succeeds does the tool proceed to [`TwoPhaseCommit::commit`] them.
/// If any object fails in either phase, the run terminates immediately and all
/// objects are [`TwoPhaseCommit::rollback`]-ed.
///
/// `None` entries are skipped, mirroring Java's tolerance of `null` elements.
///
/// Equivalent to `org.apache.lucene.index.TwoPhaseCommitTool.execute`.
///
/// # Errors
///
/// - [`TwoPhaseCommitError::PrepareCommitFail`] if any object fails to prepare.
/// - [`TwoPhaseCommitError::CommitFail`] if any object fails to commit.
///
/// Note that an object may fail to commit after others have already committed
/// successfully. This tool still issues a rollback on them, but depending on the
/// implementation that may have no effect.
///
/// # Divergence: the boundary of the rollback guarantee
///
/// Java catches `Throwable` (`TwoPhaseCommitTool.java:96` and `:109`), so
/// *anything* thrown by `prepareCommit()` or `commit()` — including unchecked
/// runtime failures and `Error`s — triggers the rollback of every object.
///
/// Rucene's guarantee covers every failure reported as
/// [`Err`](std::result::Result::Err), which is the counterpart of Java's checked
/// `IOException` and the only failure channel the [`TwoPhaseCommit`] trait
/// defines. It does **not** cover a `panic!` inside an implementation: the
/// unwind passes straight through `execute`, so no object is rolled back.
///
/// Catching the panic is deliberately not done. `catch_unwind` cannot catch an
/// abort (`panic = "abort"`, or a panic while panicking), so it would buy an
/// incomplete guarantee; and a panic in Rust means a broken invariant, at which
/// point calling `rollback()` on the very implementation that just broke is more
/// likely to corrupt the index than to save it. An implementation that wants the
/// rollback must report its failure as `Err`, not panic.
pub fn execute(
    objects: &[Option<&dyn TwoPhaseCommit>],
) -> std::result::Result<(), TwoPhaseCommitError> {
    // First, all should successfully prepare_commit().
    for tpc in objects.iter().flatten() {
        if let Err(cause) = tpc.prepare_commit() {
            // The first object that fails rolls back all of them.
            rollback_all(objects);
            return Err(TwoPhaseCommitError::PrepareCommitFail {
                object: format!("{tpc:?}"),
                cause: Box::new(cause),
            });
        }
    }

    // If all successfully prepared, attempt the actual commit().
    for tpc in objects.iter().flatten() {
        if let Err(cause) = tpc.commit() {
            // The first object that fails rolls back all of them.
            rollback_all(objects);
            return Err(TwoPhaseCommitError::CommitFail {
                object: format!("{tpc:?}"),
                cause: Box::new(cause),
            });
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Test support
// -----------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    //! In-memory, deletable [`IndexCommit`] shared by the commit and
    //! deletion-policy tests.
    //!
    //! `StandardDirectoryReader.ReaderCommit` refuses deletion (exactly as in
    //! Java), and the commit point that honours it — `IndexFileDeleter`'s —
    //! belongs to a later porting task, so the policy tests need their own
    //! deletable commit point.

    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use super::IndexCommit;
    use crate::error::Result;
    use crate::store::Directory;

    /// A commit point backed purely by memory, whose [`IndexCommit::delete`]
    /// records the request instead of touching the file system.
    pub(crate) struct TestCommit {
        directory: Arc<dyn Directory>,
        generation: i64,
        files: HashSet<String>,
        user_data: HashMap<String, String>,
        segment_count: i32,
        deleted: AtomicBool,
    }

    impl std::fmt::Debug for TestCommit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // `dyn Directory` is not `Debug`, so the identity is reported
            // through the generation, which is what the tests inspect.
            f.debug_struct("TestCommit")
                .field("generation", &self.generation)
                .field("deleted", &self.deleted.load(AtomicOrdering::SeqCst))
                .finish()
        }
    }

    impl TestCommit {
        /// Creates a commit point for `generation` referencing a single
        /// synthetic segment file.
        pub(crate) fn at_generation(
            directory: Arc<dyn Directory>,
            generation: i64,
        ) -> Arc<dyn IndexCommit> {
            let files = HashSet::from([
                format!("segments_{generation}"),
                format!("_{generation}.si"),
            ]);
            Self::with_details(directory, generation, files, HashMap::new(), 1)
        }

        /// Creates a fully specified commit point.
        pub(crate) fn with_details(
            directory: Arc<dyn Directory>,
            generation: i64,
            files: HashSet<String>,
            user_data: HashMap<String, String>,
            segment_count: i32,
        ) -> Arc<dyn IndexCommit> {
            Arc::new(Self {
                directory,
                generation,
                files,
                user_data,
                segment_count,
                deleted: AtomicBool::new(false),
            })
        }
    }

    impl IndexCommit for TestCommit {
        fn get_segments_file_name(&self) -> String {
            format!("segments_{}", self.generation)
        }

        fn get_file_names(&self) -> Result<HashSet<String>> {
            Ok(self.files.clone())
        }

        fn get_directory(&self) -> Arc<dyn Directory> {
            Arc::clone(&self.directory)
        }

        fn delete(&self) -> Result<()> {
            self.deleted.store(true, AtomicOrdering::SeqCst);
            Ok(())
        }

        fn is_deleted(&self) -> bool {
            self.deleted.load(AtomicOrdering::SeqCst)
        }

        fn get_segment_count(&self) -> i32 {
            self.segment_count
        }

        fn get_generation(&self) -> i64 {
            self.generation
        }

        fn get_user_data(&self) -> Result<HashMap<String, String>> {
            Ok(self.user_data.clone())
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::test_support::TestCommit;
    use super::*;
    use crate::store::RamDirectory;
    use std::cell::RefCell;

    fn ram_directory() -> Arc<dyn Directory> {
        Arc::new(RamDirectory::default())
    }

    // -------------------------------------------------------------------------
    // IndexCommit: equality, hashing and ordering
    // -------------------------------------------------------------------------

    #[test]
    fn commits_are_equal_when_directory_and_generation_match() {
        let dir = ram_directory();
        let a = TestCommit::at_generation(Arc::clone(&dir), 3);
        let b = TestCommit::at_generation(Arc::clone(&dir), 3);

        assert!(a.commit_equals(b.as_ref()));
        assert!(b.commit_equals(a.as_ref()));
        assert_eq!(a.commit_hash(), b.commit_hash());
    }

    #[test]
    fn commits_differ_when_generation_differs() {
        let dir = ram_directory();
        let a = TestCommit::at_generation(Arc::clone(&dir), 3);
        let b = TestCommit::at_generation(Arc::clone(&dir), 4);

        assert!(!a.commit_equals(b.as_ref()));
        assert_ne!(a.commit_hash(), b.commit_hash());
    }

    #[test]
    fn commits_differ_when_directory_differs() {
        let a = TestCommit::at_generation(ram_directory(), 7);
        let b = TestCommit::at_generation(ram_directory(), 7);

        assert!(!a.commit_equals(b.as_ref()));
    }

    #[test]
    fn compare_to_orders_by_generation() {
        let dir = ram_directory();
        let older = TestCommit::at_generation(Arc::clone(&dir), 1);
        let newer = TestCommit::at_generation(Arc::clone(&dir), 2);

        assert_eq!(older.compare_to(newer.as_ref()).unwrap(), Ordering::Less);
        assert_eq!(newer.compare_to(older.as_ref()).unwrap(), Ordering::Greater);
        assert_eq!(older.compare_to(older.as_ref()).unwrap(), Ordering::Equal);
    }

    #[test]
    fn compare_to_rejects_commits_from_different_directories() {
        let a = TestCommit::at_generation(ram_directory(), 1);
        let b = TestCommit::at_generation(ram_directory(), 2);

        let err = a.compare_to(b.as_ref()).unwrap_err();
        assert!(
            matches!(err, LuceneError::UnsupportedOperation(ref msg)
                if msg.contains("different Directory instances")),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn commits_sort_from_oldest_to_newest() {
        let dir = ram_directory();
        let mut commits = [
            TestCommit::at_generation(Arc::clone(&dir), 5),
            TestCommit::at_generation(Arc::clone(&dir), 1),
            TestCommit::at_generation(Arc::clone(&dir), 3),
        ];
        commits.sort_by(|a, b| a.compare_to(b.as_ref()).expect("same directory"));

        let generations: Vec<i64> = commits.iter().map(|c| c.get_generation()).collect();
        assert_eq!(generations, vec![1, 3, 5]);
    }

    #[test]
    fn java_long_hash_code_matches_reference_values() {
        // Long.hashCode(v) == (int) (v ^ (v >>> 32)).
        assert_eq!(java_long_hash_code(0), 0);
        assert_eq!(java_long_hash_code(1), 1);
        assert_eq!(java_long_hash_code(-1), 0);
        assert_eq!(java_long_hash_code(i64::MAX), 0x8000_0000);
        assert_eq!(java_long_hash_code(1_i64 << 32), 1);
    }

    #[test]
    fn default_commit_accessors_match_java_defaults() {
        let dir = ram_directory();
        let commit = TestCommit::at_generation(Arc::clone(&dir), 9);

        assert_eq!(commit.get_segments_file_name(), "segments_9");
        assert_eq!(commit.get_generation(), 9);
        assert_eq!(commit.get_segment_count(), 1);
        assert!(commit.get_file_names().unwrap().contains("segments_9"));
        assert!(commit.get_user_data().unwrap().is_empty());
        assert!(commit.get_reader().is_none());
        assert!(!commit.is_deleted());
    }

    // -------------------------------------------------------------------------
    // TwoPhaseCommitTool
    // -------------------------------------------------------------------------

    /// Records the calls made on it and optionally fails a given phase.
    struct RecordingTwoPhaseCommit {
        name: &'static str,
        fail_prepare: bool,
        fail_commit: bool,
        log: RefCell<Vec<String>>,
    }

    impl RecordingTwoPhaseCommit {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                fail_prepare: false,
                fail_commit: false,
                log: RefCell::new(Vec::new()),
            }
        }

        fn failing_prepare(name: &'static str) -> Self {
            Self {
                fail_prepare: true,
                ..Self::new(name)
            }
        }

        fn failing_commit(name: &'static str) -> Self {
            Self {
                fail_commit: true,
                ..Self::new(name)
            }
        }

        fn calls(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    impl Debug for RecordingTwoPhaseCommit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "RecordingTwoPhaseCommit({})", self.name)
        }
    }

    impl TwoPhaseCommit for RecordingTwoPhaseCommit {
        fn prepare_commit(&self) -> Result<i64> {
            self.log.borrow_mut().push("prepare".to_string());
            if self.fail_prepare {
                return Err(LuceneError::Io(io::Error::other("prepare boom")));
            }
            Ok(1)
        }

        fn commit(&self) -> Result<i64> {
            self.log.borrow_mut().push("commit".to_string());
            if self.fail_commit {
                return Err(LuceneError::Io(io::Error::other("commit boom")));
            }
            Ok(2)
        }

        fn rollback(&self) -> Result<()> {
            self.log.borrow_mut().push("rollback".to_string());
            Ok(())
        }
    }

    #[test]
    fn execute_prepares_all_then_commits_all() {
        let a = RecordingTwoPhaseCommit::new("a");
        let b = RecordingTwoPhaseCommit::new("b");

        execute(&[Some(&a as &dyn TwoPhaseCommit), Some(&b)]).unwrap();

        assert_eq!(a.calls(), vec!["prepare", "commit"]);
        assert_eq!(b.calls(), vec!["prepare", "commit"]);
    }

    #[test]
    fn execute_skips_none_entries() {
        let a = RecordingTwoPhaseCommit::new("a");

        execute(&[None, Some(&a as &dyn TwoPhaseCommit), None]).unwrap();

        assert_eq!(a.calls(), vec!["prepare", "commit"]);
    }

    #[test]
    fn execute_with_no_objects_is_a_no_op() {
        execute(&[]).unwrap();
        execute(&[None, None]).unwrap();
    }

    #[test]
    fn execute_rolls_everything_back_when_prepare_fails() {
        let a = RecordingTwoPhaseCommit::new("a");
        let b = RecordingTwoPhaseCommit::failing_prepare("b");
        let c = RecordingTwoPhaseCommit::new("c");

        let err = execute(&[Some(&a as &dyn TwoPhaseCommit), Some(&b), Some(&c)]).unwrap_err();

        assert!(matches!(err, TwoPhaseCommitError::PrepareCommitFail { .. }));
        assert_eq!(err.object(), "RecordingTwoPhaseCommit(b)");
        assert!(err.to_string().contains("prepareCommit() failed on"));
        assert!(err.cause().to_string().contains("prepare boom"));

        // Nobody committed, and the object after the failure was never prepared.
        assert_eq!(a.calls(), vec!["prepare", "rollback"]);
        assert_eq!(b.calls(), vec!["prepare", "rollback"]);
        assert_eq!(c.calls(), vec!["rollback"]);
    }

    #[test]
    fn execute_rolls_everything_back_when_commit_fails() {
        let a = RecordingTwoPhaseCommit::new("a");
        let b = RecordingTwoPhaseCommit::failing_commit("b");
        let c = RecordingTwoPhaseCommit::new("c");

        let err = execute(&[Some(&a as &dyn TwoPhaseCommit), Some(&b), Some(&c)]).unwrap_err();

        assert!(matches!(err, TwoPhaseCommitError::CommitFail { .. }));
        assert_eq!(err.object(), "RecordingTwoPhaseCommit(b)");
        assert!(err.to_string().contains("commit() failed on"));
        assert!(err.cause().to_string().contains("commit boom"));

        // Every object prepared; `a` committed before `b` failed; `c` never did.
        assert_eq!(a.calls(), vec!["prepare", "commit", "rollback"]);
        assert_eq!(b.calls(), vec!["prepare", "commit", "rollback"]);
        assert_eq!(c.calls(), vec!["prepare", "rollback"]);
    }

    #[test]
    fn execute_rollback_ignores_rollback_failures() {
        /// Fails both `commit` and `rollback`.
        #[derive(Debug)]
        struct Stubborn;

        impl TwoPhaseCommit for Stubborn {
            fn prepare_commit(&self) -> Result<i64> {
                Ok(0)
            }

            fn commit(&self) -> Result<i64> {
                Err(LuceneError::Io(io::Error::other("commit boom")))
            }

            fn rollback(&self) -> Result<()> {
                Err(LuceneError::Io(io::Error::other("rollback boom")))
            }
        }

        let other = RecordingTwoPhaseCommit::new("other");
        let err = execute(&[Some(&Stubborn as &dyn TwoPhaseCommit), Some(&other)]).unwrap_err();

        // The rollback failure is swallowed; the commit failure is reported and
        // every remaining object still gets rolled back.
        assert!(matches!(err, TwoPhaseCommitError::CommitFail { .. }));
        assert_eq!(other.calls(), vec!["prepare", "rollback"]);
    }

    #[test]
    fn two_phase_commit_error_converts_into_an_io_lucene_error() {
        let b = RecordingTwoPhaseCommit::failing_prepare("b");
        let err = execute(&[Some(&b as &dyn TwoPhaseCommit)]).unwrap_err();

        let lucene: LuceneError = err.into();
        assert!(matches!(lucene, LuceneError::Io(_)), "{lucene}");
        assert!(lucene.to_string().contains("prepareCommit() failed on"));
    }
}
