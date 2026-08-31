//! Index deletion policies ported from `org.apache.lucene.index`.
//!
//! A deletion policy decides when older [`IndexCommit`] points are removed from
//! an index directory. `IndexWriter` calls [`IndexDeletionPolicy::on_init`] once
//! when it opens an index and [`IndexDeletionPolicy::on_commit`] after every
//! commit, handing over the list of commits sorted from oldest to newest; the
//! policy expresses its decision by calling [`IndexCommit::delete`].
//!
//! Equivalent to:
//! - `org.apache.lucene.index.IndexDeletionPolicy`
//! - `org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy`
//! - `org.apache.lucene.index.KeepLastNCommitsDeletionPolicy`
//! - `org.apache.lucene.index.NoDeletionPolicy`
//! - `org.apache.lucene.index.SnapshotDeletionPolicy`
//! - `org.apache.lucene.index.PersistentSnapshotDeletionPolicy`

#![deny(unsafe_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::codecs::codec_util;
use crate::error::{LuceneError, Result};
use crate::index::index_commit::IndexCommit;
use crate::index::index_writer_config::OpenMode;
use crate::store::{Directory, DEFAULT_IO_CONTEXT};

// -----------------------------------------------------------------------------
// IndexDeletionPolicy
// -----------------------------------------------------------------------------

/// Policy for the deletion of stale [`IndexCommit`] points.
///
/// Set an implementation on
/// [`IndexWriterConfig::set_index_deletion_policy`](crate::index::IndexWriterConfig::set_index_deletion_policy)
/// to customise when older point-in-time commits are removed from the index
/// directory. The default policy is [`KeepOnlyLastCommitDeletionPolicy`], which
/// removes old commits as soon as a new commit is done.
///
/// One expected use case — and the reason the abstraction exists — is to work
/// around index directories accessed over file systems such as NFS, which do
/// not provide the "delete on last close" semantics that Lucene's point-in-time
/// search normally relies on. A custom policy such as "a commit is only removed
/// once it has been stale for more than X minutes" gives readers time to
/// refresh to the new commit before the writer removes the old ones, at the cost
/// of extra storage.
///
/// Equivalent to `org.apache.lucene.index.IndexDeletionPolicy`.
///
/// # Interior mutability
///
/// Java's `IndexDeletionPolicy` methods are plain instance methods on an object
/// the writer owns. Rucene shares the policy as
/// `Arc<dyn IndexDeletionPolicy>` (see
/// [`IndexWriterConfig`](crate::index::IndexWriterConfig)), so both methods take
/// `&self`; stateful policies such as [`SnapshotDeletionPolicy`] use interior
/// mutability, exactly as Java uses `synchronized`.
pub trait IndexDeletionPolicy: Send + Sync + Debug {
    /// Called once when a writer is first instantiated, to give the policy a
    /// chance to remove old commit points.
    ///
    /// The writer locates all index commits present in the index directory and
    /// calls this method. The policy may delete some of them by calling
    /// [`IndexCommit::delete`].
    ///
    /// Note that the last commit is the most recent one, i.e. the "front index
    /// state". Be careful not to delete it unless you know exactly what you are
    /// doing and can afford to lose the index content.
    ///
    /// `commits` is sorted by age: element `0` is the oldest commit. For a new
    /// index this method is invoked with an empty slice.
    ///
    /// Equivalent to `IndexDeletionPolicy.onInit(List)`.
    ///
    /// # Errors
    ///
    /// Returns an error if a commit point cannot be deleted.
    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()>;

    /// Called each time the writer completes a commit, to give the policy a
    /// chance to remove old commit points.
    ///
    /// This is only called when `IndexWriter::commit` or `IndexWriter::close`
    /// is called, and possibly not at all if `IndexWriter::rollback` is called.
    ///
    /// Note that the last commit is the most recent one, i.e. the "front index
    /// state". Be careful not to delete it.
    ///
    /// `commits` is sorted by age: element `0` is the oldest commit.
    ///
    /// Equivalent to `IndexDeletionPolicy.onCommit(List)`.
    ///
    /// # Errors
    ///
    /// Returns an error if a commit point cannot be deleted.
    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()>;
}

// -----------------------------------------------------------------------------
// KeepOnlyLastCommitDeletionPolicy
// -----------------------------------------------------------------------------

/// Keeps only the most recent commit and immediately removes all prior commits
/// after a new commit is done. This is the default deletion policy.
///
/// Equivalent to `org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KeepOnlyLastCommitDeletionPolicy;

impl KeepOnlyLastCommitDeletionPolicy {
    /// Creates the policy.
    ///
    /// Equivalent to the sole `KeepOnlyLastCommitDeletionPolicy()` constructor.
    pub fn new() -> Self {
        Self
    }
}

impl IndexDeletionPolicy for KeepOnlyLastCommitDeletionPolicy {
    /// Deletes all commits except the most recent one.
    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // Note that `commits.len()` should normally be 1.
        self.on_commit(commits)
    }

    /// Deletes all commits except the most recent one.
    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // Note that `commits.len()` should normally be 2 (when not called by
        // `on_init` above).
        delete_all_but_last_n(commits, 1)
    }
}

// -----------------------------------------------------------------------------
// KeepLastNCommitsDeletionPolicy
// -----------------------------------------------------------------------------

/// Keeps the last `N` commits and removes all prior commits after a new commit
/// is done.
///
/// This policy is useful for maintaining a history of recent commits while
/// still managing index size.
///
/// Equivalent to `org.apache.lucene.index.KeepLastNCommitsDeletionPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepLastNCommitsDeletionPolicy {
    num_commits_to_keep: usize,
}

impl KeepLastNCommitsDeletionPolicy {
    /// Creates a policy that retains the `num_commits_to_keep` most recent
    /// commits.
    ///
    /// Equivalent to `KeepLastNCommitsDeletionPolicy(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `num_commits_to_keep` is not
    /// positive, mirroring the Java `IllegalArgumentException`.
    pub fn new(num_commits_to_keep: i32) -> Result<Self> {
        if num_commits_to_keep <= 0 {
            return Err(LuceneError::IllegalArgument(
                "number of recent commits to keep must be positive".to_string(),
            ));
        }
        Ok(Self {
            num_commits_to_keep: num_commits_to_keep as usize,
        })
    }

    /// Returns the number of most recent commits this policy retains.
    pub fn num_commits_to_keep(&self) -> i32 {
        self.num_commits_to_keep as i32
    }
}

impl IndexDeletionPolicy for KeepLastNCommitsDeletionPolicy {
    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        self.on_commit(commits)
    }

    /// Deletes all but the last `N` commits.
    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // The commits slice is already sorted from oldest to newest.
        delete_all_but_last_n(commits, self.num_commits_to_keep)
    }
}

/// Deletes every commit except the `keep` most recent ones.
///
/// Shared by [`KeepOnlyLastCommitDeletionPolicy`] and
/// [`KeepLastNCommitsDeletionPolicy`], whose Java bodies are the same loop with
/// a different bound.
fn delete_all_but_last_n(commits: &[Arc<dyn IndexCommit>], keep: usize) -> Result<()> {
    for commit in commits.iter().take(commits.len().saturating_sub(keep)) {
        commit.delete()?;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// NoDeletionPolicy
// -----------------------------------------------------------------------------

/// Keeps all index commits around, never deleting them.
///
/// This policy is a singleton, reachable through [`NoDeletionPolicy::instance`].
///
/// Equivalent to `org.apache.lucene.index.NoDeletionPolicy`, whose constructor
/// is private for the same reason: the type is stateless, so a single shared
/// instance is enough.
#[derive(Debug)]
pub struct NoDeletionPolicy {
    /// Prevents construction outside this module, mirroring Java's private
    /// constructor.
    _private: (),
}

static NO_DELETION_POLICY: LazyLock<Arc<NoDeletionPolicy>> =
    LazyLock::new(|| Arc::new(NoDeletionPolicy { _private: () }));

impl NoDeletionPolicy {
    /// Returns the single instance of this policy.
    ///
    /// Equivalent to `NoDeletionPolicy.INSTANCE`.
    pub fn instance() -> Arc<dyn IndexDeletionPolicy> {
        Arc::clone(&NO_DELETION_POLICY) as Arc<dyn IndexDeletionPolicy>
    }
}

impl IndexDeletionPolicy for NoDeletionPolicy {
    fn on_init(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// SnapshotDeletionPolicy
// -----------------------------------------------------------------------------

/// Mutable state shared between a [`SnapshotDeletionPolicy`] and the
/// [`SnapshotCommitPoint`]s it hands to the wrapped policy.
///
/// In Java this state lives in the outer `SnapshotDeletionPolicy` instance and
/// the inner class reaches it through `SnapshotDeletionPolicy.this`; the shared
/// `Arc<Mutex<_>>` is the Rust equivalent of that outer reference plus the
/// `synchronized` monitor.
#[derive(Debug, Default)]
struct SnapshotState {
    /// Records how many snapshots are held against each commit generation.
    ///
    /// A `BTreeMap` is used instead of Java's `HashMap` so that the persisted
    /// `snapshots_N` file (see [`PersistentSnapshotDeletionPolicy`]) has a
    /// deterministic entry order: ascending commit generation.
    ///
    /// Entry order is not part of the file format — both readers rebuild a map
    /// — but it does mean the two implementations only emit the *same bytes*
    /// when Java's `HashMap` happens to iterate in ascending order too. Java
    /// walks its buckets, i.e. `generation & (capacity - 1)`, so `{2, 4}` comes
    /// out as `[2, 4]` while `{2, 17}` comes out as `[17, 2]`. See the
    /// "Interoperability" section on [`PersistentSnapshotDeletionPolicy`].
    ref_counts: BTreeMap<i64, i32>,
    /// Maps a generation to its commit point.
    index_commits: BTreeMap<i64, Arc<dyn IndexCommit>>,
    /// The most recently committed commit point.
    last_commit: Option<Arc<dyn IndexCommit>>,
    /// Used to detect misuse: `snapshot`/`release` before `on_init`.
    init_called: bool,
}

/// Wraps any other [`IndexDeletionPolicy`] and adds the ability to hold and
/// later release snapshots of an index.
///
/// While a snapshot is held, `IndexWriter` will not remove any file associated
/// with it, even if the index is otherwise being actively and arbitrarily
/// changed. Because an arbitrary policy is wrapped, you keep the freedom to use
/// whatever deletion policy you would normally want for your index.
///
/// This class maintains all snapshots in memory, so the information is not
/// persisted and not protected against system failures. If persistence matters,
/// use [`PersistentSnapshotDeletionPolicy`].
///
/// Equivalent to `org.apache.lucene.index.SnapshotDeletionPolicy`.
///
/// # Locking
///
/// Every method Java marks `synchronized` — `onInit`, `onCommit`, `snapshot`,
/// `release`, `releaseGen`, `incRef`, `getSnapshots`, `getSnapshotCount` and
/// `getIndexCommit` — shares one monitor, so none of them can interleave with
/// another. Rucene reproduces that monitor with a dedicated *operation* mutex
/// that each of those entry points holds for the whole operation, **including
/// the call into the wrapped `primary` policy**. Without it, `snapshot()` could
/// run in the window between `primary.on_commit` deleting the previous commit
/// and this policy publishing the new one, and would then pin — and report as
/// snapshotted — a commit that is already gone.
///
/// A second, shorter-lived mutex protects the mutable [`SnapshotState`], which
/// is shared with the [`SnapshotCommitPoint`]s handed to the wrapped policy.
/// The lock order is always *operation → state*, never the reverse:
/// [`SnapshotCommitPoint::delete`](IndexCommit::delete) — the only entry point
/// reached from inside the critical section, because the wrapped policy calls
/// it — takes the state mutex alone. That is the Rust counterpart of Java's
/// `synchronized (SnapshotDeletionPolicy.this)` inside
/// `SnapshotCommitPoint.delete()` re-entering the monitor the outer `onCommit`
/// already holds.
///
/// # Reentrancy
///
/// Java's monitor is reentrant, so a wrapped policy could in principle call
/// back into `snapshot()` from `onCommit`. `std::sync::Mutex` is not reentrant,
/// so a `primary` policy that re-enters this policy from `on_init`/`on_commit`
/// is **not supported** and will deadlock instead of returning; no
/// Lucene-provided policy — and no Rucene policy — does so. Calling
/// [`IndexCommit::delete`] on the commits handed to the primary, which is the
/// whole point of the wrapper, is explicitly supported.
pub struct SnapshotDeletionPolicy {
    /// The wrapped policy, applied to non-snapshotted commits.
    primary: Arc<dyn IndexDeletionPolicy>,
    /// Stands in for Java's `synchronized` monitor: serialises whole policy
    /// operations against each other.
    operation: Mutex<()>,
    state: Arc<Mutex<SnapshotState>>,
}

impl Debug for SnapshotDeletionPolicy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("SnapshotDeletionPolicy");
        out.field("primary", &self.primary);
        // `try_lock` keeps `Debug` deadlock-free even when it is reached from
        // inside a critical section, for instance through an info-stream log.
        match self.state.try_lock() {
            Ok(state) => out
                .field("snapshotted_generations", &state.ref_counts.len())
                .field(
                    "last_commit_generation",
                    &state.last_commit.as_ref().map(|c| c.get_generation()),
                ),
            Err(_) => out.field("state", &"<locked>"),
        }
        .finish()
    }
}

impl SnapshotDeletionPolicy {
    /// Creates a snapshot policy wrapping `primary`.
    ///
    /// Equivalent to `SnapshotDeletionPolicy(IndexDeletionPolicy)`.
    pub fn new(primary: Arc<dyn IndexDeletionPolicy>) -> Self {
        Self {
            primary,
            operation: Mutex::new(()),
            state: Arc::new(Mutex::new(SnapshotState::default())),
        }
    }

    /// Acquires the operation mutex, i.e. enters the critical section Java
    /// enters when it calls a `synchronized` method on this policy.
    ///
    /// It must be held for the whole operation, the call into the wrapped
    /// `primary` policy included, and is always taken *before*
    /// [`SnapshotDeletionPolicy::lock`].
    ///
    /// Poisoning is recovered from for the same reason as in
    /// [`SnapshotDeletionPolicy::lock`]; the guarded value is `()`, so there is
    /// no state a panic could have torn.
    pub(crate) fn lock_operation(&self) -> MutexGuard<'_, ()> {
        self.operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Locks the shared state, recovering from poisoning.
    ///
    /// The guarded state is a pair of plain maps with no cross-field invariant
    /// that a panic could leave inconsistent, and every section that holds it is
    /// short and free of calls into foreign code. Recovering therefore keeps the
    /// policy usable instead of turning a transient panic into a permanently
    /// failing deletion policy — which would mean the writer could never delete
    /// a commit again.
    fn lock(&self) -> MutexGuard<'_, SnapshotState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Snapshots the last commit and returns it.
    ///
    /// Once a commit is snapshotted it is protected from deletion, as long as
    /// this policy is in use. The snapshot is removed by calling
    /// [`SnapshotDeletionPolicy::release`] followed by
    /// `IndexWriter::delete_unused_files`.
    ///
    /// While the snapshot is held the files it references will not be deleted,
    /// consuming additional disk space. Taking a snapshot at a particularly bad
    /// time (say, just before a force merge) can in the worst case consume an
    /// extra 1x of the total index size until the snapshot is released.
    ///
    /// Equivalent to `SnapshotDeletionPolicy.snapshot()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if this instance is not being used
    /// by an `IndexWriter` (i.e. [`IndexDeletionPolicy::on_init`] was never
    /// called), or if the index does not have any commit yet.
    pub fn snapshot(&self) -> Result<Arc<dyn IndexCommit>> {
        let _operation = self.lock_operation();
        self.snapshot_locked()
    }

    /// [`SnapshotDeletionPolicy::snapshot`] for a caller that already holds the
    /// operation mutex, which is how [`PersistentSnapshotDeletionPolicy`]
    /// composes `snapshot` with persisting the result under the single monitor
    /// Java inherits.
    pub(crate) fn snapshot_locked(&self) -> Result<Arc<dyn IndexCommit>> {
        let mut state = self.lock();
        Self::check_init_called(&state)?;
        let last_commit = match state.last_commit.clone() {
            Some(commit) => commit,
            // No commit yet, e.g. this is a new IndexWriter.
            None => {
                return Err(LuceneError::IllegalState(
                    "No index commit to snapshot".to_string(),
                ))
            }
        };
        Self::inc_ref_state(&mut state, last_commit.as_ref());
        Ok(last_commit)
    }

    /// Releases a snapshotted commit previously returned by
    /// [`SnapshotDeletionPolicy::snapshot`].
    ///
    /// Equivalent to `SnapshotDeletionPolicy.release(IndexCommit)`.
    ///
    /// # Errors
    ///
    /// - [`LuceneError::IllegalState`] if this instance is not being used by an
    ///   `IndexWriter`.
    /// - [`LuceneError::IllegalArgument`] if the commit's generation is not
    ///   currently snapshotted.
    pub fn release(&self, commit: &dyn IndexCommit) -> Result<()> {
        // Java's `release(IndexCommit)` is `synchronized` and delegates to the
        // plain `releaseGen`; the operation mutex is taken there.
        self.release_gen(commit.get_generation())
    }

    /// [`SnapshotDeletionPolicy::release`] for a caller that already holds the
    /// operation mutex.
    pub(crate) fn release_locked(&self, commit: &dyn IndexCommit) -> Result<()> {
        self.release_gen_locked(commit.get_generation())
    }

    /// Releases a snapshot by generation.
    ///
    /// Equivalent to the `protected` `SnapshotDeletionPolicy.releaseGen(long)`
    /// as reached through the `synchronized` `release(IndexCommit)`; it is
    /// `pub(crate)` here because Rust has no `protected` and Lucene exposes no
    /// public release-by-generation on the in-memory policy —
    /// [`PersistentSnapshotDeletionPolicy::release_gen`] is the public one.
    ///
    /// # Errors
    ///
    /// - [`LuceneError::IllegalState`] if this instance is not being used by an
    ///   `IndexWriter`.
    /// - [`LuceneError::IllegalArgument`] if `gen` is not currently snapshotted.
    pub(crate) fn release_gen(&self, gen: i64) -> Result<()> {
        let _operation = self.lock_operation();
        self.release_gen_locked(gen)
    }

    /// [`SnapshotDeletionPolicy::release_gen`] for a caller that already holds
    /// the operation mutex.
    pub(crate) fn release_gen_locked(&self, gen: i64) -> Result<()> {
        let mut state = self.lock();
        Self::check_init_called(&state)?;
        let Some(ref_count) = state.ref_counts.get(&gen).copied() else {
            return Err(LuceneError::IllegalArgument(format!(
                "commit gen={gen} is not currently snapshotted"
            )));
        };
        debug_assert!(ref_count > 0);
        let ref_count = ref_count - 1;
        if ref_count == 0 {
            state.ref_counts.remove(&gen);
            state.index_commits.remove(&gen);
        } else {
            state.ref_counts.insert(gen, ref_count);
        }
        Ok(())
    }

    /// Increments the reference count for `ic`.
    ///
    /// Equivalent to the `protected` `SnapshotDeletionPolicy.incRef(IndexCommit)`,
    /// which Java marks `synchronized`; the only caller is
    /// [`PersistentSnapshotDeletionPolicy`], which already holds the operation
    /// mutex, so this method takes the state mutex alone.
    pub(crate) fn inc_ref(&self, ic: &dyn IndexCommit) {
        let mut state = self.lock();
        Self::inc_ref_state(&mut state, ic);
    }

    fn inc_ref_state(state: &mut SnapshotState, ic: &dyn IndexCommit) {
        let gen = ic.get_generation();
        let ref_count = match state.ref_counts.get(&gen).copied() {
            Some(count) => count,
            None => {
                // Faithful to Java, which registers `lastCommit` here rather
                // than `ic`. Java would store a `null` value when there is no
                // last commit; Rucene simply records no entry, which is
                // observably the same (`get_index_commit` yields nothing) while
                // keeping the map free of null-like states.
                if let Some(last_commit) = state.last_commit.clone() {
                    state.index_commits.insert(gen, last_commit);
                }
                0
            }
        };
        state.ref_counts.insert(gen, ref_count + 1);
    }

    /// Returns all commit points held by at least one snapshot.
    ///
    /// Equivalent to `SnapshotDeletionPolicy.getSnapshots()`.
    pub fn get_snapshots(&self) -> Vec<Arc<dyn IndexCommit>> {
        // `synchronized` in Java: the answer must not be read from a policy
        // operation that is still half-applied.
        let _operation = self.lock_operation();
        self.lock().index_commits.values().cloned().collect()
    }

    /// Returns the total number of snapshots currently held.
    ///
    /// Equivalent to `SnapshotDeletionPolicy.getSnapshotCount()`.
    pub fn get_snapshot_count(&self) -> i32 {
        // `synchronized` in Java: see `get_snapshots`.
        let _operation = self.lock_operation();
        // Java accumulates into an `int`, which wraps silently on overflow.
        // Reaching it needs more than 2^31 live snapshots, each one pinning at
        // least a `segments_N` file, so it cannot happen in practice; wrapping
        // rather than `sum()` keeps the debug build from panicking where Java
        // would quietly wrap.
        self.lock()
            .ref_counts
            .values()
            .fold(0i32, |total, count| total.wrapping_add(*count))
    }

    /// Retrieves a commit point from its generation, or `None` if that commit
    /// is not currently snapshotted.
    ///
    /// Equivalent to `SnapshotDeletionPolicy.getIndexCommit(long)`.
    pub fn get_index_commit(&self, gen: i64) -> Option<Arc<dyn IndexCommit>> {
        // `synchronized` in Java: see `get_snapshots`.
        let _operation = self.lock_operation();
        self.lock().index_commits.get(&gen).cloned()
    }

    /// Returns a copy of the current reference counts, keyed by commit
    /// generation.
    ///
    /// Java exposes the map itself as a `protected` field; Rucene returns a
    /// snapshot so callers cannot mutate the policy's state behind its back.
    ///
    /// Callers must already hold the operation mutex — this takes the state
    /// mutex alone, so that [`PersistentSnapshotDeletionPolicy::persist`] can
    /// read the counts from inside its critical section.
    fn ref_counts(&self) -> BTreeMap<i64, i32> {
        self.lock().ref_counts.clone()
    }

    /// Reproduces the `initCalled` guard of `SnapshotDeletionPolicy.snapshot()`
    /// and `releaseGen()`.
    ///
    /// The wording deliberately differs from Java's. Java tells the caller to
    /// use `writer.getConfig().getIndexDeletionPolicy()` and cast the result
    /// back to `SnapshotDeletionPolicy`;
    /// [`IndexWriterConfig::index_deletion_policy`](crate::index::IndexWriterConfig::index_deletion_policy)
    /// returns `Arc<dyn IndexDeletionPolicy>`, which Rust cannot downcast to the
    /// concrete policy, so following Java's advice verbatim would be
    /// impossible. The only workable route is the one the message describes:
    /// keep the `Arc<SnapshotDeletionPolicy>` you built and pass a clone of it
    /// to the config — `Arc` makes the two views the same object, so this is
    /// exactly what Java's advice achieves.
    fn check_init_called(state: &SnapshotState) -> Result<()> {
        if state.init_called {
            return Ok(());
        }
        Err(LuceneError::IllegalState(
            "this instance is not being used by IndexWriter; keep your own \
             Arc<SnapshotDeletionPolicy> and hand a clone of it to \
             IndexWriterConfig::set_index_deletion_policy, then snapshot through \
             the Arc you kept"
                .to_string(),
        ))
    }

    /// Wraps each commit as a [`SnapshotCommitPoint`].
    fn wrap_commits(&self, commits: &[Arc<dyn IndexCommit>]) -> Vec<Arc<dyn IndexCommit>> {
        commits
            .iter()
            .map(|ic| {
                Arc::new(SnapshotCommitPoint {
                    cp: Arc::clone(ic),
                    state: Arc::clone(&self.state),
                }) as Arc<dyn IndexCommit>
            })
            .collect()
    }
}

impl SnapshotDeletionPolicy {
    /// [`IndexDeletionPolicy::on_init`] for a caller that already holds the
    /// operation mutex.
    pub(crate) fn on_init_locked(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // Java sets `initCalled` before delegating, so a `primary` that calls
        // back into `snapshot()` observes the same state.
        self.lock().init_called = true;

        // The wrapped policy is invoked while the *operation* mutex is held —
        // that is what makes the whole callback atomic against `snapshot()` and
        // `release()`, exactly as Java's monitor does — but *without* the state
        // mutex: the policy calls `delete()` on the wrappers, which takes the
        // state mutex, and `std::sync::Mutex` is not reentrant.
        self.primary.on_init(&self.wrap_commits(commits))?;

        let mut state = self.lock();
        for commit in commits {
            let gen = commit.get_generation();
            if state.ref_counts.contains_key(&gen) {
                state.index_commits.insert(gen, Arc::clone(commit));
            }
        }
        if let Some(last) = commits.last() {
            state.last_commit = Some(Arc::clone(last));
        }
        Ok(())
    }

    /// [`IndexDeletionPolicy::on_commit`] for a caller that already holds the
    /// operation mutex.
    pub(crate) fn on_commit_locked(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // See `on_init_locked` for which lock is held across this call and why.
        self.primary.on_commit(&self.wrap_commits(commits))?;

        // Java does `lastCommit = commits.get(commits.size() - 1)`, which throws
        // `IndexOutOfBoundsException` on an empty list. Rucene reports the same
        // misuse as a recoverable error, after the primary policy has run, so
        // the observable side effects stay identical.
        let Some(last) = commits.last() else {
            return Err(LuceneError::IllegalArgument(
                "on_commit was called with no commits".to_string(),
            ));
        };
        self.lock().last_commit = Some(Arc::clone(last));
        Ok(())
    }
}

impl IndexDeletionPolicy for SnapshotDeletionPolicy {
    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        let _operation = self.lock_operation();
        self.on_init_locked(commits)
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        let _operation = self.lock_operation();
        self.on_commit_locked(commits)
    }
}

/// Wraps a commit point and prevents it from being deleted while snapshotted.
///
/// Equivalent to the private inner class
/// `SnapshotDeletionPolicy.SnapshotCommitPoint`.
struct SnapshotCommitPoint {
    /// The commit point being protected from deletion.
    cp: Arc<dyn IndexCommit>,
    state: Arc<Mutex<SnapshotState>>,
}

impl Debug for SnapshotCommitPoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Mirrors `SnapshotDeletionPolicy.SnapshotCommitPoint.toString()`.
        write!(
            f,
            "SnapshotDeletionPolicy.SnapshotCommitPoint({:?})",
            self.cp
        )
    }
}

impl IndexCommit for SnapshotCommitPoint {
    fn get_segments_file_name(&self) -> String {
        self.cp.get_segments_file_name()
    }

    fn get_file_names(&self) -> Result<HashSet<String>> {
        self.cp.get_file_names()
    }

    fn get_directory(&self) -> Arc<dyn Directory> {
        self.cp.get_directory()
    }

    fn delete(&self) -> Result<()> {
        let snapshotted = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.ref_counts.contains_key(&self.cp.get_generation())
        };
        // Suppress the delete request if this commit point is currently
        // snapshotted. The wrapped `delete()` runs after the guard is dropped so
        // that a foreign implementation cannot deadlock against the policy.
        if !snapshotted {
            self.cp.delete()?;
        }
        Ok(())
    }

    fn is_deleted(&self) -> bool {
        self.cp.is_deleted()
    }

    fn get_segment_count(&self) -> i32 {
        self.cp.get_segment_count()
    }

    fn get_generation(&self) -> i64 {
        self.cp.get_generation()
    }

    fn get_user_data(&self) -> Result<HashMap<String, String>> {
        self.cp.get_user_data()
    }

    // `get_reader` is deliberately *not* delegated: Java's SnapshotCommitPoint
    // does not override `getReader()` either, so it yields the base `null`.
}

// -----------------------------------------------------------------------------
// PersistentSnapshotDeletionPolicy
// -----------------------------------------------------------------------------

/// Prefix used for the snapshot save file.
///
/// Equivalent to `PersistentSnapshotDeletionPolicy.SNAPSHOTS_PREFIX`.
pub const SNAPSHOTS_PREFIX: &str = "snapshots_";

/// Codec name written into the `snapshots_N` header.
const CODEC_NAME: &str = "snapshots";
/// First — and, in Lucene 10.5.0, only — version of the `snapshots_N` format.
const VERSION_START: i32 = 0;
/// Version written by this implementation.
const VERSION_CURRENT: i32 = VERSION_START;

/// A [`SnapshotDeletionPolicy`] with a persistence layer, so that snapshots can
/// be maintained across the life of an application.
///
/// The snapshots are persisted in a [`Directory`] and are committed as soon as
/// [`PersistentSnapshotDeletionPolicy::snapshot`] or
/// [`PersistentSnapshotDeletionPolicy::release`] is called.
///
/// Sharing a `PersistentSnapshotDeletionPolicy` that writes to the same
/// directory across several `IndexWriter`s will corrupt snapshots. Make sure
/// every writer has its own policy and that they all write to a different
/// directory. It is fine to use the same directory that holds the index.
///
/// Equivalent to `org.apache.lucene.index.PersistentSnapshotDeletionPolicy`.
///
/// # File format
///
/// The save file is named `snapshots_<gen>` and contains, in this order:
///
/// 1. a codec header (`CodecUtil.writeHeader`) with codec name `snapshots` and
///    version `0`;
/// 2. a `VInt` with the number of snapshotted generations;
/// 3. for each of them, a `VLong` commit generation followed by a `VInt`
///    reference count.
///
/// There is no footer and no checksum, matching Lucene 10.5.0 exactly.
///
/// # Interoperability
///
/// Compatibility with Lucene 10.5.0 holds at the level of the **format**, not
/// of the exact byte string: entry order is not part of the format, and neither
/// implementation depends on it, because both rebuild a map from the entries.
/// Rucene emits the entries ordered by ascending commit generation; Java emits
/// them in `HashMap` bucket order, which is `generation & (capacity - 1)` and
/// therefore only coincides with ascending order for some sets of generations
/// (`{2, 4}` does, `{2, 17}` does not — it comes out as `[17, 2]`).
///
/// So a Rucene-written file is always readable by Lucene and vice versa, and
/// the two files are byte-identical exactly when both orders agree.
pub struct PersistentSnapshotDeletionPolicy {
    /// The in-memory snapshot bookkeeping this policy persists.
    ///
    /// Java uses inheritance (`extends SnapshotDeletionPolicy`); Rucene uses
    /// composition and forwards the public surface, which keeps the base
    /// policy's invariants encapsulated.
    inner: SnapshotDeletionPolicy,
    dir: Arc<dyn Directory>,
    /// Generation of the next `snapshots_N` file to write.
    ///
    /// Java inherits the `synchronized` methods of `SnapshotDeletionPolicy`, so
    /// `snapshot`, `release`, `onInit` and `onCommit` all share *one* monitor
    /// with the base class. Rucene reproduces that by taking
    /// [`SnapshotDeletionPolicy::lock_operation`] on the wrapped policy, so this
    /// mutex only provides interior mutability for the counter. The lock order
    /// is always *inner operation → this → inner state*, so it is total.
    next_write_gen: Mutex<i64>,
}

impl Debug for PersistentSnapshotDeletionPolicy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut out = f.debug_struct("PersistentSnapshotDeletionPolicy");
        out.field("inner", &self.inner);
        // See `Debug for SnapshotDeletionPolicy` for why this is `try_lock`.
        match self.next_write_gen.try_lock() {
            Ok(gen) if *gen == 0 => out.field("last_save_file", &None::<String>),
            Ok(gen) => out.field("last_save_file", &format!("{SNAPSHOTS_PREFIX}{}", *gen - 1)),
            Err(_) => out.field("last_save_file", &"<locked>"),
        }
        .finish()
    }
}

impl PersistentSnapshotDeletionPolicy {
    /// Wraps `primary` and persists snapshot information into `dir`, using
    /// [`OpenMode::CREATE_OR_APPEND`].
    ///
    /// Equivalent to
    /// `PersistentSnapshotDeletionPolicy(IndexDeletionPolicy, Directory)`.
    ///
    /// # Errors
    ///
    /// See [`PersistentSnapshotDeletionPolicy::with_open_mode`].
    pub fn new(primary: Arc<dyn IndexDeletionPolicy>, dir: Arc<dyn Directory>) -> Result<Self> {
        Self::with_open_mode(primary, dir, OpenMode::CREATE_OR_APPEND)
    }

    /// Wraps `primary` and persists snapshot information into `dir`.
    ///
    /// [`OpenMode::CREATE`] deletes all existing snapshot information
    /// immediately; the other modes load the snapshot information already
    /// stored in `dir`.
    ///
    /// Equivalent to
    /// `PersistentSnapshotDeletionPolicy(IndexDeletionPolicy, Directory, OpenMode)`.
    ///
    /// # Errors
    ///
    /// - [`LuceneError::IllegalState`] if `mode` is [`OpenMode::APPEND`] and no
    ///   snapshots are stored in `dir`.
    /// - [`LuceneError::IllegalArgument`] if `dir` holds a file whose name
    ///   starts with `snapshots_` but does not end in a valid generation,
    ///   mirroring the Java `NumberFormatException`.
    /// - Any I/O error raised while listing, deleting or reading files.
    pub fn with_open_mode(
        primary: Arc<dyn IndexDeletionPolicy>,
        dir: Arc<dyn Directory>,
        mode: OpenMode,
    ) -> Result<Self> {
        let policy = Self {
            inner: SnapshotDeletionPolicy::new(primary),
            dir,
            next_write_gen: Mutex::new(0),
        };

        if mode == OpenMode::CREATE {
            policy.clear_prior_snapshots()?;
        }

        {
            let mut next_write_gen = policy.lock_persist();
            policy.load_prior_snapshots(&mut next_write_gen)?;

            if mode == OpenMode::APPEND && *next_write_gen == 0 {
                return Err(LuceneError::IllegalState(
                    "no snapshots stored in this directory".to_string(),
                ));
            }
        }

        Ok(policy)
    }

    /// Locks the persistence state, recovering from poisoning for the same
    /// reasons documented on [`SnapshotDeletionPolicy::lock`].
    fn lock_persist(&self) -> MutexGuard<'_, i64> {
        self.next_write_gen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Snapshots the last commit. Once this method returns, the snapshot
    /// information is persisted in the directory.
    ///
    /// Equivalent to `PersistentSnapshotDeletionPolicy.snapshot()`.
    ///
    /// # Errors
    ///
    /// See [`SnapshotDeletionPolicy::snapshot`], plus any I/O error raised while
    /// persisting. If persisting fails the snapshot is released again, so the
    /// in-memory state stays consistent with what is on disk.
    pub fn snapshot(&self) -> Result<Arc<dyn IndexCommit>> {
        let _operation = self.inner.lock_operation();
        let mut next_write_gen = self.lock_persist();
        let ic = self.inner.snapshot_locked()?;
        match self.persist(&mut next_write_gen) {
            Ok(()) => Ok(ic),
            Err(err) => {
                // Suppress any secondary failure so the original error is kept.
                let _ = self.inner.release_locked(ic.as_ref());
                Err(err)
            }
        }
    }

    /// Deletes a snapshotted commit. Once this method returns, the snapshot
    /// information is persisted in the directory.
    ///
    /// Equivalent to `PersistentSnapshotDeletionPolicy.release(IndexCommit)`.
    ///
    /// # Errors
    ///
    /// See [`SnapshotDeletionPolicy::release`], plus any I/O error raised while
    /// persisting. If persisting fails the reference count is restored.
    pub fn release(&self, commit: &dyn IndexCommit) -> Result<()> {
        let _operation = self.inner.lock_operation();
        let mut next_write_gen = self.lock_persist();
        self.inner.release_locked(commit)?;
        match self.persist(&mut next_write_gen) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.inner.inc_ref(commit);
                Err(err)
            }
        }
    }

    /// Deletes a snapshotted commit by generation. Once this method returns,
    /// the snapshot information is persisted in the directory.
    ///
    /// Equivalent to `PersistentSnapshotDeletionPolicy.release(long)`.
    ///
    /// # Errors
    ///
    /// - [`LuceneError::IllegalState`] if this instance is not being used by an
    ///   `IndexWriter`.
    /// - [`LuceneError::IllegalArgument`] if `gen` is not currently snapshotted.
    /// - Any I/O error raised while persisting. Java does not restore the
    ///   reference count when persisting fails here, and neither does Rucene.
    pub fn release_gen(&self, gen: i64) -> Result<()> {
        let _operation = self.inner.lock_operation();
        let mut next_write_gen = self.lock_persist();
        self.inner.release_gen_locked(gen)?;
        self.persist(&mut next_write_gen)
    }

    /// Returns the file name the snapshots are currently saved to, or `None` if
    /// no snapshots have been saved.
    ///
    /// Equivalent to `PersistentSnapshotDeletionPolicy.getLastSaveFile()`.
    pub fn get_last_save_file(&self) -> Option<String> {
        let next_write_gen = *self.lock_persist();
        if next_write_gen == 0 {
            None
        } else {
            Some(format!("{SNAPSHOTS_PREFIX}{}", next_write_gen - 1))
        }
    }

    /// Returns all commit points held by at least one snapshot.
    ///
    /// Inherited from [`SnapshotDeletionPolicy::get_snapshots`].
    pub fn get_snapshots(&self) -> Vec<Arc<dyn IndexCommit>> {
        self.inner.get_snapshots()
    }

    /// Returns the total number of snapshots currently held.
    ///
    /// Inherited from [`SnapshotDeletionPolicy::get_snapshot_count`].
    pub fn get_snapshot_count(&self) -> i32 {
        self.inner.get_snapshot_count()
    }

    /// Retrieves a commit point from its generation, or `None` if that commit
    /// is not currently snapshotted.
    ///
    /// Inherited from [`SnapshotDeletionPolicy::get_index_commit`].
    pub fn get_index_commit(&self, gen: i64) -> Option<Arc<dyn IndexCommit>> {
        self.inner.get_index_commit(gen)
    }

    /// Writes the current reference counts to a new `snapshots_N` file, fsyncs
    /// it, and removes the previous one.
    ///
    /// Equivalent to the private `PersistentSnapshotDeletionPolicy.persist()`.
    ///
    /// # Deliberate divergence: a save file that fails to close
    ///
    /// Java writes the file inside a try-with-resources and sets its `success`
    /// flag as the **last statement of the block body**
    /// (`PersistentSnapshotDeletionPolicy.java:174-186`), so the flag is already
    /// `true` when `out.close()` runs. If closing fails — a disk that fills up
    /// while the last buffered bytes are flushed is the realistic case — the
    /// `finally` sees `success == true` and does **not** delete the file, while
    /// the exception still propagates before `nextWriteGen++` (`:196`). Both
    /// implementations create the file with `CREATE_NEW` semantics
    /// (`FSDirectory.java:230`), so the leftover `snapshots_N` blocks the next
    /// attempt at the very same generation.
    ///
    /// Measured against the reference (fixture shape `persist-close-failure`),
    /// Lucene 10.5.0 behaves like this:
    ///
    /// | attempt | outcome                        | `snapshots_0` afterwards |
    /// |---------|--------------------------------|--------------------------|
    /// | 1st     | `IOException` from `close()`   | present                  |
    /// | 2nd     | `FileAlreadyExistsException`   | deleted by its `finally` |
    /// | 3rd     | succeeds                       | present and valid        |
    ///
    /// So the cost is one **extra** failed `snapshot()`/`release()`, not a
    /// permanently broken policy — the second attempt's own cleanup path
    /// unblocks the generation again. Rucene instead treats a failed `close()`
    /// like any other write failure and removes the unusable file immediately,
    /// so the very next attempt succeeds and no caller ever sees the spurious
    /// `AlreadyExists` failure.
    ///
    /// The divergence is confined to **error recovery**. The bytes of a
    /// successfully written file, its name and the generation sequence are
    /// unchanged, so a `snapshots_N` produced by Rucene is still read by Java
    /// and vice versa. See
    /// `persistent_snapshot_recovers_when_the_save_file_fails_to_close` and the
    /// `persist_close_failure_diverges_from_lucene_only_in_recovery`
    /// portability test.
    fn persist(&self, next_write_gen: &mut i64) -> Result<()> {
        let file_name = format!("{SNAPSHOTS_PREFIX}{}", *next_write_gen);
        let ref_counts = self.inner.ref_counts();

        let written = (|| -> Result<()> {
            let mut out = self.dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
            let body = (|| -> Result<()> {
                codec_util::write_header(out.as_mut(), CODEC_NAME, VERSION_CURRENT)?;
                out.write_v_int(ref_counts.len() as i32)?;
                for (commit_gen, ref_count) in &ref_counts {
                    out.write_v_long(*commit_gen)?;
                    out.write_v_int(*ref_count)?;
                }
                Ok(())
            })();
            match body {
                Ok(()) => out.close(),
                Err(err) => {
                    let _ = out.close();
                    Err(err)
                }
            }
        })();

        if written.is_err() {
            // Exception OK: the partial file may not even exist.
            let _ = self.dir.delete_file(&file_name);
            return written;
        }

        self.dir.sync(std::slice::from_ref(&file_name))?;

        if *next_write_gen > 0 {
            let last_save_file = format!("{SNAPSHOTS_PREFIX}{}", *next_write_gen - 1);
            // Exception OK: likely it did not exist.
            let _ = self.dir.delete_file(&last_save_file);
        }

        *next_write_gen += 1;
        Ok(())
    }

    /// Removes every `snapshots_*` file from the directory.
    ///
    /// Equivalent to the private
    /// `PersistentSnapshotDeletionPolicy.clearPriorSnapshots()`.
    fn clear_prior_snapshots(&self) -> Result<()> {
        for file in self.dir.list_all()? {
            if file.starts_with(SNAPSHOTS_PREFIX) {
                self.dir.delete_file(&file)?;
            }
        }
        Ok(())
    }

    /// Reads the snapshot information already stored in the directory.
    ///
    /// Equivalent to the private
    /// `PersistentSnapshotDeletionPolicy.loadPriorSnapshots()`, including its
    /// error handling, which treats two failures very differently:
    ///
    /// - a save file whose *content* cannot be parsed (`catch (IOException)`,
    ///   `PersistentSnapshotDeletionPolicy.java:234-239`) leaves the reference
    ///   counts empty and is **not** reported, as long as some save file was
    ///   reached; only when no save file at all could be read is the first such
    ///   error propagated;
    /// - a failure to *close* the save file (`finally { in.close(); }`,
    ///   `:240-242`) propagates immediately, before `genLoaded` and the
    ///   reference counts are updated, so the policy refuses to open rather
    ///   than starting from a state it is not sure it read completely.
    fn load_prior_snapshots(&self, next_write_gen: &mut i64) -> Result<()> {
        let mut gen_loaded: i64 = -1;
        let mut first_error: Option<LuceneError> = None;
        let mut snapshot_files: Vec<String> = Vec::new();

        for file in self.dir.list_all()? {
            if !file.starts_with(SNAPSHOTS_PREFIX) {
                continue;
            }
            let gen: i64 = file[SNAPSHOTS_PREFIX.len()..].parse().map_err(|_| {
                LuceneError::IllegalArgument(format!(
                    "not a valid snapshots file name: {file}; expected {SNAPSHOTS_PREFIX}<generation>"
                ))
            })?;
            if gen_loaded != -1 && gen <= gen_loaded {
                continue;
            }
            snapshot_files.push(file.clone());

            let mut loaded: BTreeMap<i64, i32> = BTreeMap::new();
            let mut input = self.dir.open_input(&file, &*DEFAULT_IO_CONTEXT)?;
            let read = (|| -> Result<()> {
                codec_util::check_header(input.as_mut(), CODEC_NAME, VERSION_START, VERSION_START)?;
                let count = input.read_v_int()?;
                for _ in 0..count {
                    let commit_gen = input.read_v_long()?;
                    let ref_count = input.read_v_int()?;
                    loaded.insert(commit_gen, ref_count);
                }
                Ok(())
            })();
            // Java's `finally { in.close(); }` sits *outside* the `catch`, so a
            // close failure propagates instead of being folded into the saved
            // parse error — and it does so before `genLoaded` is advanced.
            input.close()?;
            if let Err(err) = read {
                // Save the first parse error and report it only if nothing loads.
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }

            gen_loaded = gen;
            let mut state = self.inner.lock();
            state.ref_counts.clear();
            state.ref_counts.extend(loaded);
        }

        if gen_loaded == -1 {
            // Nothing was loaded...
            if let Some(err) = first_error {
                // ... not for lack of trying.
                return Err(err);
            }
        } else {
            if snapshot_files.len() > 1 {
                // Remove any broken / old snapshot files.
                let current = format!("{SNAPSHOTS_PREFIX}{gen_loaded}");
                for file in &snapshot_files {
                    if *file != current {
                        let _ = self.dir.delete_file(file);
                    }
                }
            }
            *next_write_gen = 1 + gen_loaded;
        }

        Ok(())
    }
}

impl IndexDeletionPolicy for PersistentSnapshotDeletionPolicy {
    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        // Java inherits the `synchronized` methods, so `onInit` shares one
        // monitor with `snapshot()`/`release()`; taking the wrapped policy's
        // operation mutex — rather than a second one of our own — is what makes
        // it the *same* monitor.
        let _operation = self.inner.lock_operation();
        self.inner.on_init_locked(commits)
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        let _operation = self.inner.lock_operation();
        self.inner.on_commit_locked(commits)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::index_commit::test_support::TestCommit;
    use crate::store::RamDirectory;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::Condvar;
    use std::time::{Duration, Instant};

    fn ram_directory() -> Arc<dyn Directory> {
        Arc::new(RamDirectory::default())
    }

    fn commits(dir: &Arc<dyn Directory>, generations: &[i64]) -> Vec<Arc<dyn IndexCommit>> {
        generations
            .iter()
            .map(|gen| TestCommit::at_generation(Arc::clone(dir), *gen))
            .collect()
    }

    fn deleted_flags(commits: &[Arc<dyn IndexCommit>]) -> Vec<bool> {
        commits.iter().map(|c| c.is_deleted()).collect()
    }

    fn read_file(dir: &dyn Directory, name: &str) -> Vec<u8> {
        let len = dir.file_length(name).unwrap() as usize;
        let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
        let mut bytes = vec![0u8; len];
        input.read_bytes(&mut bytes, 0, len).unwrap();
        input.close().unwrap();
        bytes
    }

    fn snapshot_files(dir: &dyn Directory) -> Vec<String> {
        let mut files: Vec<String> = dir
            .list_all()
            .unwrap()
            .into_iter()
            .filter(|f| f.starts_with(SNAPSHOTS_PREFIX))
            .collect();
        files.sort();
        files
    }

    // -------------------------------------------------------------------------
    // KeepOnlyLastCommitDeletionPolicy
    // -------------------------------------------------------------------------

    #[test]
    fn keep_only_last_deletes_every_commit_but_the_newest() {
        let dir = ram_directory();
        let commits = commits(&dir, &[1, 2, 3]);
        let policy = KeepOnlyLastCommitDeletionPolicy::new();

        policy.on_commit(&commits).unwrap();

        assert_eq!(deleted_flags(&commits), vec![true, true, false]);
    }

    #[test]
    fn keep_only_last_on_init_behaves_like_on_commit() {
        let dir = ram_directory();
        let commits = commits(&dir, &[7, 8]);

        KeepOnlyLastCommitDeletionPolicy.on_init(&commits).unwrap();

        assert_eq!(deleted_flags(&commits), vec![true, false]);
    }

    #[test]
    fn keep_only_last_keeps_a_lone_commit() {
        let dir = ram_directory();
        let commits = commits(&dir, &[1]);

        KeepOnlyLastCommitDeletionPolicy::new()
            .on_commit(&commits)
            .unwrap();

        assert_eq!(deleted_flags(&commits), vec![false]);
    }

    #[test]
    fn keep_only_last_tolerates_an_index_without_commits() {
        let policy = KeepOnlyLastCommitDeletionPolicy::new();
        policy.on_init(&[]).unwrap();
        policy.on_commit(&[]).unwrap();
    }

    #[test]
    fn keep_only_last_propagates_a_delete_failure() {
        /// Refuses deletion, exactly like `ReaderCommit` does.
        struct UndeletableCommit(Arc<dyn Directory>);

        impl Debug for UndeletableCommit {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_str("UndeletableCommit")
            }
        }

        impl IndexCommit for UndeletableCommit {
            fn get_segments_file_name(&self) -> String {
                "segments_1".to_string()
            }
            fn get_file_names(&self) -> Result<HashSet<String>> {
                Ok(HashSet::new())
            }
            fn get_directory(&self) -> Arc<dyn Directory> {
                Arc::clone(&self.0)
            }
            fn delete(&self) -> Result<()> {
                Err(LuceneError::UnsupportedOperation(
                    "This IndexCommit does not support deletions".to_string(),
                ))
            }
            fn is_deleted(&self) -> bool {
                false
            }
            fn get_segment_count(&self) -> i32 {
                1
            }
            fn get_generation(&self) -> i64 {
                1
            }
            fn get_user_data(&self) -> Result<HashMap<String, String>> {
                Ok(HashMap::new())
            }
        }

        let dir = ram_directory();
        let commits: Vec<Arc<dyn IndexCommit>> = vec![
            Arc::new(UndeletableCommit(Arc::clone(&dir))),
            TestCommit::at_generation(Arc::clone(&dir), 2),
        ];

        let err = KeepOnlyLastCommitDeletionPolicy::new()
            .on_commit(&commits)
            .unwrap_err();
        assert!(matches!(err, LuceneError::UnsupportedOperation(_)), "{err}");
    }

    // -------------------------------------------------------------------------
    // KeepLastNCommitsDeletionPolicy
    // -------------------------------------------------------------------------

    #[test]
    fn keep_last_n_deletes_only_the_oldest_commits() {
        let dir = ram_directory();
        let commits = commits(&dir, &[1, 2, 3, 4, 5]);

        KeepLastNCommitsDeletionPolicy::new(2)
            .unwrap()
            .on_commit(&commits)
            .unwrap();

        assert_eq!(
            deleted_flags(&commits),
            vec![true, true, true, false, false]
        );
    }

    #[test]
    fn keep_last_n_keeps_everything_when_n_exceeds_the_commit_count() {
        let dir = ram_directory();
        let commits = commits(&dir, &[1, 2]);

        let policy = KeepLastNCommitsDeletionPolicy::new(10).unwrap();
        policy.on_init(&commits).unwrap();
        policy.on_commit(&commits).unwrap();

        assert_eq!(deleted_flags(&commits), vec![false, false]);
        assert_eq!(policy.num_commits_to_keep(), 10);
    }

    #[test]
    fn keep_last_n_of_one_matches_keep_only_last() {
        let dir = ram_directory();
        let a = commits(&dir, &[1, 2, 3]);
        let b = commits(&dir, &[1, 2, 3]);

        KeepLastNCommitsDeletionPolicy::new(1)
            .unwrap()
            .on_commit(&a)
            .unwrap();
        KeepOnlyLastCommitDeletionPolicy::new()
            .on_commit(&b)
            .unwrap();

        assert_eq!(deleted_flags(&a), deleted_flags(&b));
    }

    #[test]
    fn keep_last_n_rejects_a_non_positive_count() {
        for n in [0, -1, i32::MIN] {
            let err = KeepLastNCommitsDeletionPolicy::new(n).unwrap_err();
            assert!(
                matches!(err, LuceneError::IllegalArgument(ref m)
                    if m.contains("must be positive")),
                "unexpected error for n={n}: {err}"
            );
        }
    }

    #[test]
    fn keep_last_n_tolerates_an_index_without_commits() {
        let policy = KeepLastNCommitsDeletionPolicy::new(3).unwrap();
        policy.on_init(&[]).unwrap();
        policy.on_commit(&[]).unwrap();
    }

    // -------------------------------------------------------------------------
    // NoDeletionPolicy
    // -------------------------------------------------------------------------

    #[test]
    fn no_deletion_policy_never_deletes() {
        let dir = ram_directory();
        let commits = commits(&dir, &[1, 2, 3]);
        let policy = NoDeletionPolicy::instance();

        policy.on_init(&commits).unwrap();
        policy.on_commit(&commits).unwrap();
        policy.on_init(&[]).unwrap();

        assert_eq!(deleted_flags(&commits), vec![false, false, false]);
    }

    #[test]
    fn no_deletion_policy_is_a_singleton() {
        assert!(Arc::ptr_eq(
            &NoDeletionPolicy::instance(),
            &NoDeletionPolicy::instance()
        ));
    }

    // -------------------------------------------------------------------------
    // SnapshotDeletionPolicy
    // -------------------------------------------------------------------------

    fn snapshot_policy() -> SnapshotDeletionPolicy {
        SnapshotDeletionPolicy::new(Arc::new(KeepOnlyLastCommitDeletionPolicy::new()))
    }

    #[test]
    fn snapshot_requires_the_policy_to_be_in_use_by_a_writer() {
        let policy = snapshot_policy();

        let err = policy.snapshot().unwrap_err();
        // The advice must be something the caller can actually act on: Rust
        // cannot downcast the `Arc<dyn IndexDeletionPolicy>` that
        // `IndexWriterConfig::index_deletion_policy` returns, so — unlike Java —
        // the message must point at keeping the concrete `Arc`.
        assert!(
            matches!(err, LuceneError::IllegalState(ref m)
                if m.contains("not being used by IndexWriter")
                    && m.contains("Arc<SnapshotDeletionPolicy>")
                    && m.contains("set_index_deletion_policy")),
            "{err}"
        );

        let err = policy.release_gen(1).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalState(_)), "{err}");
    }

    #[test]
    fn snapshot_requires_at_least_one_commit() {
        let policy = snapshot_policy();
        policy.on_init(&[]).unwrap();

        let err = policy.snapshot().unwrap_err();
        assert!(
            matches!(err, LuceneError::IllegalState(ref m)
                if m == "No index commit to snapshot"),
            "{err}"
        );
        assert_eq!(policy.get_snapshot_count(), 0);
        assert!(policy.get_snapshots().is_empty());
    }

    #[test]
    fn snapshot_pins_a_commit_across_later_commits() {
        let dir = ram_directory();
        let policy = snapshot_policy();

        let first = commits(&dir, &[1, 2]);
        policy.on_init(&first).unwrap();
        // The wrapped KeepOnlyLast policy already dropped the oldest commit.
        assert_eq!(deleted_flags(&first), vec![true, false]);

        let snapshotted = policy.snapshot().unwrap();
        assert_eq!(snapshotted.get_generation(), 2);
        assert_eq!(policy.get_snapshot_count(), 1);

        // A new commit arrives: without the snapshot, gen 2 would be deleted.
        let third = TestCommit::at_generation(Arc::clone(&dir), 3);
        let second_round = vec![Arc::clone(&first[1]), Arc::clone(&third)];
        policy.on_commit(&second_round).unwrap();

        assert!(!first[1].is_deleted(), "snapshotted commit must survive");
        assert!(!third.is_deleted());

        // Releasing it lets the next commit remove it.
        policy.release(snapshotted.as_ref()).unwrap();
        assert_eq!(policy.get_snapshot_count(), 0);

        let fourth = TestCommit::at_generation(Arc::clone(&dir), 4);
        policy
            .on_commit(&[Arc::clone(&first[1]), Arc::clone(&third), fourth])
            .unwrap();
        assert!(first[1].is_deleted(), "released commit must be deletable");
        assert!(third.is_deleted());
    }

    #[test]
    fn snapshotting_the_same_commit_twice_reference_counts_it() {
        let dir = ram_directory();
        let policy = snapshot_policy();
        let initial = commits(&dir, &[5]);
        policy.on_init(&initial).unwrap();

        let a = policy.snapshot().unwrap();
        let b = policy.snapshot().unwrap();
        assert_eq!(policy.get_snapshot_count(), 2);
        assert_eq!(policy.get_snapshots().len(), 1);
        assert!(a.commit_equals(b.as_ref()));

        // One release still leaves the commit pinned.
        policy.release(a.as_ref()).unwrap();
        assert_eq!(policy.get_snapshot_count(), 1);
        assert!(policy.get_index_commit(5).is_some());

        let newer = TestCommit::at_generation(Arc::clone(&dir), 6);
        policy
            .on_commit(&[Arc::clone(&initial[0]), Arc::clone(&newer)])
            .unwrap();
        assert!(!initial[0].is_deleted());

        // The second release drops it entirely.
        policy.release(b.as_ref()).unwrap();
        assert_eq!(policy.get_snapshot_count(), 0);
        assert!(policy.get_index_commit(5).is_none());
        assert!(policy.get_snapshots().is_empty());
    }

    #[test]
    fn releasing_the_same_snapshot_twice_is_rejected() {
        let dir = ram_directory();
        let policy = snapshot_policy();
        policy.on_init(&commits(&dir, &[1])).unwrap();

        let snapshotted = policy.snapshot().unwrap();
        policy.release(snapshotted.as_ref()).unwrap();

        let err = policy.release(snapshotted.as_ref()).unwrap_err();
        assert!(
            matches!(err, LuceneError::IllegalArgument(ref m)
                if m == "commit gen=1 is not currently snapshotted"),
            "{err}"
        );
    }

    #[test]
    fn releasing_a_commit_that_was_never_snapshotted_is_rejected() {
        let dir = ram_directory();
        let policy = snapshot_policy();
        policy.on_init(&commits(&dir, &[1])).unwrap();

        let stranger = TestCommit::at_generation(Arc::clone(&dir), 42);
        let err = policy.release(stranger.as_ref()).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)), "{err}");
    }

    #[test]
    fn a_snapshot_survives_the_commit_disappearing_from_the_index() {
        let dir = ram_directory();
        let policy = snapshot_policy();

        let initial = commits(&dir, &[1]);
        policy.on_init(&initial).unwrap();
        let snapshotted = policy.snapshot().unwrap();

        // The writer moves on and the snapshotted generation is no longer part
        // of the commit list handed to the policy.
        let later = commits(&dir, &[2, 3]);
        policy.on_commit(&later).unwrap();

        assert!(!initial[0].is_deleted());
        let held = policy.get_index_commit(1).expect("still snapshotted");
        assert!(held.commit_equals(snapshotted.as_ref()));
        assert_eq!(policy.get_snapshot_count(), 1);

        // Releasing a commit that is gone from the index still works.
        policy.release(snapshotted.as_ref()).unwrap();
        assert_eq!(policy.get_snapshot_count(), 0);
    }

    #[test]
    fn on_init_reattaches_commits_to_reference_counts_loaded_beforehand() {
        // This is the path `PersistentSnapshotDeletionPolicy` relies on: the
        // reference counts exist before the writer reports its commits.
        let dir = ram_directory();
        let policy = snapshot_policy();
        policy.lock().ref_counts.insert(2, 1);

        let initial = commits(&dir, &[1, 2, 3]);
        policy.on_init(&initial).unwrap();

        assert!(policy.get_index_commit(2).is_some());
        assert!(policy.get_index_commit(1).is_none());
        // Generation 2 was protected even though KeepOnlyLast asked for it.
        assert_eq!(deleted_flags(&initial), vec![true, false, false]);
    }

    #[test]
    fn on_commit_without_commits_is_reported() {
        let policy = snapshot_policy();
        policy.on_init(&[]).unwrap();

        let err = policy.on_commit(&[]).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)), "{err}");
    }

    #[test]
    fn snapshot_commit_point_delegates_and_renders_like_java() {
        let dir = ram_directory();
        let policy = snapshot_policy();
        let initial = TestCommit::with_details(
            Arc::clone(&dir),
            4,
            HashSet::from(["segments_4".to_string()]),
            HashMap::from([("k".to_string(), "v".to_string())]),
            3,
        );

        let wrapped = policy.wrap_commits(std::slice::from_ref(&initial));
        let point = &wrapped[0];

        assert_eq!(point.get_segments_file_name(), "segments_4");
        assert_eq!(point.get_generation(), 4);
        assert_eq!(point.get_segment_count(), 3);
        assert_eq!(
            point.get_file_names().unwrap(),
            initial.get_file_names().unwrap()
        );
        assert_eq!(point.get_user_data().unwrap()["k"], "v");
        assert!(Arc::ptr_eq(&point.get_directory(), &dir));
        assert!(point.get_reader().is_none());
        assert!(!point.is_deleted());
        assert!(format!("{point:?}").starts_with("SnapshotDeletionPolicy.SnapshotCommitPoint("));

        point.delete().unwrap();
        assert!(point.is_deleted());
        assert!(initial.is_deleted());
    }

    #[test]
    fn snapshot_policy_wraps_an_arbitrary_primary_policy() {
        let dir = ram_directory();
        let policy = SnapshotDeletionPolicy::new(NoDeletionPolicy::instance());
        let initial = commits(&dir, &[1, 2, 3]);

        policy.on_init(&initial).unwrap();
        policy.on_commit(&initial).unwrap();

        assert_eq!(deleted_flags(&initial), vec![false, false, false]);
        assert!(format!("{policy:?}").contains("SnapshotDeletionPolicy"));
    }

    // -------------------------------------------------------------------------
    // SnapshotDeletionPolicy: atomicity against a concurrent policy callback
    // -------------------------------------------------------------------------

    /// A one-shot latch, used to hand control back and forth between the test
    /// thread and the thread running the deletion policy.
    #[derive(Debug, Default)]
    struct Gate {
        open: Mutex<bool>,
        changed: Condvar,
    }

    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// Opens the gate, waking every waiter.
        fn open(&self) {
            *self.open.lock().unwrap_or_else(|e| e.into_inner()) = true;
            self.changed.notify_all();
        }

        /// Blocks until the gate is open.
        fn wait(&self) {
            let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            while !*open {
                open = self.changed.wait(open).unwrap_or_else(|e| e.into_inner());
            }
        }

        /// Blocks until the gate is open or `timeout` elapses; returns whether
        /// the gate opened.
        fn wait_timeout(&self, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            while !*open {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    return false;
                };
                let (guard, _) = self
                    .changed
                    .wait_timeout(open, remaining)
                    .unwrap_or_else(|e| e.into_inner());
                open = guard;
            }
            true
        }
    }

    /// Which policy callback [`BlockingPrimary`] parks in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Phase {
        OnInit,
        OnCommit,
    }

    /// A primary policy that performs its deletions and then parks, so the test
    /// can act exactly in the window between the wrapped policy finishing and
    /// [`SnapshotDeletionPolicy`] publishing its new `last_commit`.
    ///
    /// This is what `IndexWriter` does in practice: it calls `onCommit` from
    /// `IndexFileDeleter.checkpoint()` on the writer thread while the
    /// application calls `snapshot()` on another one.
    #[derive(Debug)]
    struct BlockingPrimary {
        inner: KeepOnlyLastCommitDeletionPolicy,
        phase: Phase,
        entered: Arc<Gate>,
        proceed: Arc<Gate>,
    }

    impl BlockingPrimary {
        fn park(&self, phase: Phase) {
            if self.phase == phase {
                self.entered.open();
                self.proceed.wait();
            }
        }
    }

    impl IndexDeletionPolicy for BlockingPrimary {
        fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
            self.inner.on_init(commits)?;
            self.park(Phase::OnInit);
            Ok(())
        }

        fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
            self.inner.on_commit(commits)?;
            self.park(Phase::OnCommit);
            Ok(())
        }
    }

    /// Where a probing thread parks the result of its `snapshot()` call: the
    /// generation it returned and whether that commit had already been deleted.
    type SnapshotOutcome = Arc<Mutex<Option<Result<(i64, bool)>>>>;

    /// Result of a `snapshot()` call made from another thread.
    struct SnapshotProbe {
        started: Arc<Gate>,
        done: Arc<Gate>,
        outcome: SnapshotOutcome,
        handle: std::thread::JoinHandle<()>,
    }

    /// Calls `policy.snapshot()` on a new thread, recording the generation it
    /// returned and whether that commit had already been deleted.
    fn probe_snapshot(policy: Arc<SnapshotDeletionPolicy>) -> SnapshotProbe {
        let started = Gate::new();
        let done = Gate::new();
        let outcome: SnapshotOutcome = Arc::new(Mutex::new(None));

        let handle = {
            let started = Arc::clone(&started);
            let done = Arc::clone(&done);
            let outcome = Arc::clone(&outcome);
            std::thread::spawn(move || {
                started.open();
                let result = policy
                    .snapshot()
                    .map(|commit| (commit.get_generation(), commit.is_deleted()));
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                done.open();
            })
        };

        // Only return once the thread is running, so the caller's timeout
        // measures lock contention rather than thread start-up latency.
        started.wait();
        SnapshotProbe {
            started,
            done,
            outcome,
            handle,
        }
    }

    impl SnapshotProbe {
        /// Waits up to `timeout` for the probe to finish; returns whether it did.
        fn finished_within(&self, timeout: Duration) -> bool {
            self.done.wait_timeout(timeout)
        }

        /// Joins the probing thread and returns its outcome.
        fn join(self) -> Result<(i64, bool)> {
            let _ = &self.started;
            self.handle.join().expect("probe thread must not panic");
            self.outcome
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("probe must have recorded an outcome")
        }
    }

    /// How long a probe is given to prove it is *not* blocked. It only has to
    /// be long enough for a thread that is free to run to finish a `snapshot()`
    /// call, which takes microseconds; once the policy is correct the probe can
    /// never finish within it, so the bound is not a source of flakiness.
    const CONTENTION_WINDOW: Duration = Duration::from_millis(750);

    #[test]
    fn snapshot_never_pins_a_commit_that_on_commit_already_deleted() {
        let dir = ram_directory();
        let entered = Gate::new();
        let proceed = Gate::new();
        let policy = Arc::new(SnapshotDeletionPolicy::new(Arc::new(BlockingPrimary {
            inner: KeepOnlyLastCommitDeletionPolicy::new(),
            phase: Phase::OnCommit,
            entered: Arc::clone(&entered),
            proceed: Arc::clone(&proceed),
        })));

        // Open the index on a single commit, so `last_commit` is generation 1.
        policy.on_init(&commits(&dir, &[1])).unwrap();

        // Commit generation 2; `KeepOnlyLastCommitDeletionPolicy` deletes
        // generation 1 and the primary then parks.
        let live = commits(&dir, &[1, 2]);
        let committer = {
            let policy = Arc::clone(&policy);
            let live = live.clone();
            std::thread::spawn(move || policy.on_commit(&live))
        };
        entered.wait();
        assert!(
            live[0].is_deleted(),
            "the primary policy must have deleted generation 1 before parking"
        );

        // Java holds the monitor for the whole of `onCommit`, so a concurrent
        // `snapshot()` cannot run here.
        let probe = probe_snapshot(Arc::clone(&policy));
        let ran_during_on_commit = probe.finished_within(CONTENTION_WINDOW);

        proceed.open();
        committer.join().unwrap().unwrap();
        let (generation, deleted) = probe.join().expect("snapshot must succeed");

        assert!(
            !ran_during_on_commit,
            "snapshot() ran while on_commit was in flight; the two must be mutually exclusive"
        );
        assert_eq!(
            generation, 2,
            "snapshot() must pin the commit on_commit published, not the deleted one"
        );
        assert!(!deleted, "a snapshotted commit must never be a deleted one");
        assert_eq!(policy.get_snapshot_count(), 1);
        assert!(policy.get_index_commit(2).is_some());
    }

    #[test]
    fn snapshot_never_races_on_init_into_a_missing_last_commit() {
        let dir = ram_directory();
        let entered = Gate::new();
        let proceed = Gate::new();
        let policy = Arc::new(SnapshotDeletionPolicy::new(Arc::new(BlockingPrimary {
            inner: KeepOnlyLastCommitDeletionPolicy::new(),
            phase: Phase::OnInit,
            entered: Arc::clone(&entered),
            proceed: Arc::clone(&proceed),
        })));

        // Opening an existing index: `on_init` sets `init_called`, runs the
        // primary (which deletes generation 1) and only then publishes
        // `last_commit`.
        let existing = commits(&dir, &[1, 2]);
        let initialiser = {
            let policy = Arc::clone(&policy);
            let existing = existing.clone();
            std::thread::spawn(move || policy.on_init(&existing))
        };
        entered.wait();

        let probe = probe_snapshot(Arc::clone(&policy));
        let ran_during_on_init = probe.finished_within(CONTENTION_WINDOW);

        proceed.open();
        initialiser.join().unwrap().unwrap();
        let outcome = probe.join();

        assert!(
            !ran_during_on_init,
            "snapshot() ran while on_init was in flight; the two must be mutually exclusive"
        );
        let (generation, deleted) =
            outcome.expect("snapshot() must not fail just because on_init is still running");
        assert_eq!(generation, 2);
        assert!(!deleted);
    }

    // -------------------------------------------------------------------------
    // PersistentSnapshotDeletionPolicy
    // -------------------------------------------------------------------------

    fn persistent_policy(dir: &Arc<dyn Directory>) -> PersistentSnapshotDeletionPolicy {
        PersistentSnapshotDeletionPolicy::new(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(dir),
        )
        .unwrap()
    }

    /// Builds the exact bytes Lucene 10.5.0 writes for the given entries.
    fn expected_snapshots_file(entries: &[(i64, i32)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        // CodecUtil.writeHeader: CODEC_MAGIC as big-endian int.
        bytes.extend_from_slice(&codec_util::CODEC_MAGIC.to_be_bytes());
        // DataOutput.writeString("snapshots"): VInt length then UTF-8 bytes.
        bytes.push(CODEC_NAME.len() as u8);
        bytes.extend_from_slice(CODEC_NAME.as_bytes());
        // Version, as a big-endian int.
        bytes.extend_from_slice(&VERSION_CURRENT.to_be_bytes());
        // VInt entry count.
        bytes.push(entries.len() as u8);
        for (gen, ref_count) in entries {
            // Small values encode as a single VLong/VInt byte.
            assert!((0..128).contains(gen) && (0..128).contains(ref_count));
            bytes.push(*gen as u8);
            bytes.push(*ref_count as u8);
        }
        bytes
    }

    #[test]
    fn persistent_snapshot_writes_the_lucene_file_format() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);
        assert!(policy.get_last_save_file().is_none());

        policy.on_init(&commits(&dir, &[7])).unwrap();
        policy.snapshot().unwrap();

        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_0"));
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_0"]);
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_0"),
            expected_snapshots_file(&[(7, 1)])
        );
    }

    #[test]
    fn persistent_snapshot_rotates_the_save_file() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);
        policy.on_init(&commits(&dir, &[1])).unwrap();

        let first = policy.snapshot().unwrap();
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_0"]);

        policy.snapshot().unwrap();
        // The previous generation is removed as soon as the new one is durable.
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_1"]);
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_1"),
            expected_snapshots_file(&[(1, 2)])
        );

        policy.release(first.as_ref()).unwrap();
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_2"]);
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_2"),
            expected_snapshots_file(&[(1, 1)])
        );

        policy.release_gen(1).unwrap();
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_3"]);
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_3"),
            expected_snapshots_file(&[])
        );
        assert_eq!(policy.get_snapshot_count(), 0);
    }

    #[test]
    fn persistent_snapshot_entries_are_ordered_by_generation() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);

        // Snapshot three different generations, newest first in wall-clock
        // order, to prove the file is ordered by generation and not by
        // insertion.
        policy.on_init(&commits(&dir, &[9])).unwrap();
        policy.snapshot().unwrap();
        policy.on_commit(&commits(&dir, &[9, 3])).unwrap();
        policy.snapshot().unwrap();
        policy.on_commit(&commits(&dir, &[3, 6])).unwrap();
        policy.snapshot().unwrap();

        let file = policy.get_last_save_file().unwrap();
        assert_eq!(
            read_file(dir.as_ref(), &file),
            expected_snapshots_file(&[(3, 1), (6, 1), (9, 1)])
        );
    }

    #[test]
    fn persistent_snapshot_loads_a_save_file_whose_entries_are_out_of_order() {
        let dir = ram_directory();

        // Entry order is not part of the format: Lucene writes whatever order
        // its `HashMap` iterates in, which for these generations is *not*
        // ascending (bucket order puts 17 before 2). Rucene must recover the
        // same state from either order.
        let unordered = expected_snapshots_file(&[(17, 3), (2, 1), (9, 2)]);
        let ascending = expected_snapshots_file(&[(2, 1), (9, 2), (17, 3)]);
        assert_ne!(
            unordered, ascending,
            "the two orders must really differ, otherwise this test proves nothing"
        );

        {
            let mut out = dir
                .create_output("snapshots_4", &*DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_bytes(&unordered, 0, unordered.len()).unwrap();
            out.close().unwrap();
        }

        let policy = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
            OpenMode::APPEND,
        )
        .unwrap();

        assert_eq!(policy.get_snapshot_count(), 6);
        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_4"));

        // Re-persisting normalises the order, which is what makes Rucene's own
        // output deterministic without changing what the file means.
        policy.on_init(&commits(&dir, &[9])).unwrap();
        policy.release_gen(9).unwrap();
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_5"),
            expected_snapshots_file(&[(2, 1), (9, 1), (17, 3)])
        );
    }

    #[test]
    fn persistent_snapshot_reloads_state_from_an_existing_directory() {
        let dir = ram_directory();
        {
            let policy = persistent_policy(&dir);
            policy.on_init(&commits(&dir, &[4])).unwrap();
            policy.snapshot().unwrap();
            policy.snapshot().unwrap();
            assert_eq!(policy.get_snapshot_count(), 2);
        }

        let reopened = persistent_policy(&dir);
        assert_eq!(reopened.get_snapshot_count(), 2);
        assert_eq!(
            reopened.get_last_save_file().as_deref(),
            Some("snapshots_1")
        );
        // The commit objects are only reattached once the writer reports them.
        assert!(reopened.get_index_commit(4).is_none());

        let live = commits(&dir, &[4, 5]);
        reopened.on_init(&live).unwrap();
        assert!(reopened.get_index_commit(4).is_some());
        assert!(
            !live[0].is_deleted(),
            "the reloaded snapshot must protect gen 4"
        );

        // Releasing twice clears the reloaded reference count and persists it.
        reopened.release_gen(4).unwrap();
        reopened.release_gen(4).unwrap();
        assert_eq!(reopened.get_snapshot_count(), 0);
        assert_eq!(
            read_file(dir.as_ref(), &reopened.get_last_save_file().unwrap()),
            expected_snapshots_file(&[])
        );
    }

    #[test]
    fn persistent_snapshot_reload_keeps_only_the_newest_save_file() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);
        policy.on_init(&commits(&dir, &[1])).unwrap();
        policy.snapshot().unwrap();

        // Forge two stale save files alongside the live one.
        for gen in [7_i64, 9] {
            let name = format!("{SNAPSHOTS_PREFIX}{gen}");
            let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            codec_util::write_header(out.as_mut(), CODEC_NAME, VERSION_CURRENT).unwrap();
            out.write_v_int(1).unwrap();
            out.write_v_long(gen).unwrap();
            out.write_v_int(3).unwrap();
            out.close().unwrap();
        }
        assert_eq!(
            snapshot_files(dir.as_ref()),
            vec!["snapshots_0", "snapshots_7", "snapshots_9"]
        );

        let reopened = persistent_policy(&dir);
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_9"]);
        assert_eq!(reopened.get_snapshot_count(), 3);
        assert_eq!(
            reopened.get_last_save_file().as_deref(),
            Some("snapshots_9")
        );
    }

    #[test]
    fn snapshot_count_wraps_like_java_instead_of_panicking() {
        let dir = ram_directory();

        // Reference counts are read straight off `snapshots_N` without an upper
        // bound, so a corrupt — or hostile — save file can make the total
        // overflow. Java accumulates into an `int` and wraps silently
        // (`SnapshotDeletionPolicy.java:158-164`); summing into an `i32` would
        // panic here in a debug build, turning a bad file into a crash.
        {
            let mut out = dir
                .create_output("snapshots_0", &*DEFAULT_IO_CONTEXT)
                .unwrap();
            codec_util::write_header(out.as_mut(), CODEC_NAME, VERSION_CURRENT).unwrap();
            out.write_v_int(2).unwrap();
            out.write_v_long(1).unwrap();
            out.write_v_int(i32::MAX).unwrap();
            out.write_v_long(2).unwrap();
            out.write_v_int(1).unwrap();
            out.close().unwrap();
        }

        let policy = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
            OpenMode::APPEND,
        )
        .unwrap();

        assert_eq!(policy.get_snapshot_count(), i32::MAX.wrapping_add(1));
    }

    #[test]
    fn persistent_snapshot_reload_skips_a_lower_generation_that_sorts_last() {
        let dir = ram_directory();

        // Generations of different widths: the directory listing is sorted
        // lexicographically, so `snapshots_10` is visited *before*
        // `snapshots_9`. Java behaves the same way — `dir.listAll()` is sorted
        // and the loop's guard is `genLoaded == -1 || gen > genLoaded`
        // (`PersistentSnapshotDeletionPolicy.java:225`) — so the lower
        // generation is skipped without ever entering `snapshotFiles`, and is
        // therefore left behind instead of being cleaned up.
        for (gen, ref_count) in [(9_i64, 5_i32), (10, 2)] {
            let name = format!("{SNAPSHOTS_PREFIX}{gen}");
            let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
            let bytes = expected_snapshots_file(&[(gen, ref_count)]);
            out.write_bytes(&bytes, 0, bytes.len()).unwrap();
            out.close().unwrap();
        }
        assert_eq!(
            dir.list_all()
                .unwrap()
                .iter()
                .filter(|f| f.starts_with(SNAPSHOTS_PREFIX))
                .cloned()
                .collect::<Vec<_>>(),
            vec!["snapshots_10", "snapshots_9"],
            "the listing must really put the wider name first, or this test proves nothing"
        );

        let policy = persistent_policy(&dir);

        // The state comes from generation 10, not from the file that sorts last.
        assert_eq!(policy.get_snapshot_count(), 2);
        assert!(policy.get_index_commit(9).is_none());
        assert_eq!(
            policy.get_last_save_file().as_deref(),
            Some("snapshots_10"),
            "the next save file must follow generation 10"
        );

        // `snapshots_9` is orphaned rather than deleted, exactly as in Java.
        assert_eq!(
            snapshot_files(dir.as_ref()),
            vec!["snapshots_10", "snapshots_9"]
        );

        // And the next write continues from 11, so the orphan is never reused.
        policy.on_init(&commits(&dir, &[4])).unwrap();
        policy.snapshot().unwrap();
        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_11"));
    }

    #[test]
    fn persistent_snapshot_tolerates_a_corrupt_save_file() {
        // Lucene keeps `genLoaded` even when the file fails to parse, so the
        // reference counts end up empty and the error is swallowed. Locking in
        // that behaviour keeps Rucene bug-compatible with 10.5.0.
        let dir = ram_directory();
        let mut out = dir
            .create_output("snapshots_0", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(b"not a codec header at all", 0, 25)
            .unwrap();
        out.close().unwrap();

        let policy = persistent_policy(&dir);
        assert_eq!(policy.get_snapshot_count(), 0);
        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_0"));

        // The next persist starts from the recovered generation.
        policy.on_init(&commits(&dir, &[2])).unwrap();
        policy.snapshot().unwrap();
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_1"]);
        assert_eq!(
            read_file(dir.as_ref(), "snapshots_1"),
            expected_snapshots_file(&[(2, 1)])
        );
    }

    #[test]
    fn persistent_snapshot_truncated_save_file_yields_no_snapshots() {
        let dir = ram_directory();
        let mut out = dir
            .create_output("snapshots_0", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        codec_util::write_header(out.as_mut(), CODEC_NAME, VERSION_CURRENT).unwrap();
        // Claim two entries but write only one and a half.
        out.write_v_int(2).unwrap();
        out.write_v_long(1).unwrap();
        out.close().unwrap();

        let policy = persistent_policy(&dir);
        assert_eq!(policy.get_snapshot_count(), 0);
    }

    #[test]
    fn persistent_snapshot_rejects_a_bogus_save_file_name() {
        let dir = ram_directory();
        let mut out = dir
            .create_output("snapshots_oops", &*DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_v_int(0).unwrap();
        out.close().unwrap();

        let err = PersistentSnapshotDeletionPolicy::new(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
        )
        .unwrap_err();
        assert!(
            matches!(err, LuceneError::IllegalArgument(ref m)
                if m.contains("not a valid snapshots file name")),
            "{err}"
        );
    }

    #[test]
    fn persistent_snapshot_append_mode_requires_existing_snapshots() {
        let dir = ram_directory();

        let err = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
            OpenMode::APPEND,
        )
        .unwrap_err();
        assert!(
            matches!(err, LuceneError::IllegalState(ref m)
                if m == "no snapshots stored in this directory"),
            "{err}"
        );

        // Once something is persisted, APPEND succeeds and sees the state.
        let policy = persistent_policy(&dir);
        policy.on_init(&commits(&dir, &[1])).unwrap();
        policy.snapshot().unwrap();

        let appended = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
            OpenMode::APPEND,
        )
        .unwrap();
        assert_eq!(appended.get_snapshot_count(), 1);
    }

    #[test]
    fn persistent_snapshot_create_mode_discards_prior_snapshots() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);
        policy.on_init(&commits(&dir, &[1])).unwrap();
        policy.snapshot().unwrap();
        assert_eq!(snapshot_files(dir.as_ref()), vec!["snapshots_0"]);

        let recreated = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir),
            OpenMode::CREATE,
        )
        .unwrap();

        assert!(snapshot_files(dir.as_ref()).is_empty());
        assert_eq!(recreated.get_snapshot_count(), 0);
        assert!(recreated.get_last_save_file().is_none());
    }

    #[test]
    fn persistent_snapshot_pins_commits_like_the_in_memory_policy() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);

        let initial = commits(&dir, &[1, 2]);
        policy.on_init(&initial).unwrap();
        assert_eq!(deleted_flags(&initial), vec![true, false]);

        let snapshotted = policy.snapshot().unwrap();
        let newer = TestCommit::at_generation(Arc::clone(&dir), 3);
        policy
            .on_commit(&[Arc::clone(&initial[1]), Arc::clone(&newer)])
            .unwrap();
        assert!(!initial[1].is_deleted());
        assert_eq!(policy.get_snapshots().len(), 1);

        policy.release(snapshotted.as_ref()).unwrap();
        let newest = TestCommit::at_generation(Arc::clone(&dir), 4);
        policy
            .on_commit(&[Arc::clone(&initial[1]), Arc::clone(&newer), newest])
            .unwrap();
        assert!(initial[1].is_deleted());
        assert!(format!("{policy:?}").contains("PersistentSnapshotDeletionPolicy"));
    }

    // -------------------------------------------------------------------------
    // PersistentSnapshotDeletionPolicy: recovery from a failed `persist()`
    // -------------------------------------------------------------------------

    /// An [`IndexOutput`] that reports a failure from `close()`, modelling a
    /// disk that fills up while the last buffered bytes are being flushed.
    struct FailingCloseOutput {
        inner: Box<dyn crate::store::IndexOutput>,
    }

    impl crate::store::DataOutput for FailingCloseOutput {
        fn write_byte(&mut self, b: u8) -> Result<()> {
            self.inner.write_byte(b)
        }

        fn write_bytes(&mut self, b: &[u8], offset: usize, length: usize) -> Result<()> {
            self.inner.write_bytes(b, offset, length)
        }
    }

    impl crate::store::IndexOutput for FailingCloseOutput {
        fn close(&mut self) -> Result<()> {
            // Flush what we can, exactly as a real output would, and then fail.
            self.inner.close()?;
            Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "no space left on device",
            )))
        }

        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }

        fn checksum(&self) -> Result<i64> {
            self.inner.checksum()
        }

        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }

        fn name(&self) -> &str {
            self.inner.name()
        }
    }

    /// An [`IndexInput`](crate::store::IndexInput) that reports a failure from
    /// `close()`, modelling a read that only turns out to be incomplete when
    /// the handle is released.
    struct FailingCloseInput {
        inner: Box<dyn crate::store::IndexInput>,
    }

    impl crate::store::DataInput for FailingCloseInput {
        fn read_byte(&mut self) -> Result<u8> {
            self.inner.read_byte()
        }

        fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
            self.inner.read_bytes(b, offset, len)
        }

        fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
            self.inner.skip_bytes(num_bytes)
        }
    }

    impl crate::store::IndexInput for FailingCloseInput {
        fn close(&mut self) -> Result<()> {
            self.inner.close()?;
            Err(LuceneError::Io(std::io::Error::other(
                "the save file could not be closed",
            )))
        }

        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }

        fn length(&self) -> i64 {
            self.inner.length()
        }

        fn seek(&mut self, pos: i64) -> Result<()> {
            self.inner.seek(pos)
        }

        fn slice(
            &self,
            slice_description: &str,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn crate::store::IndexInput>> {
            self.inner.slice(slice_description, offset, length)
        }

        fn clone_input(&self) -> Result<Box<dyn crate::store::IndexInput>> {
            self.inner.clone_input()
        }

        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }
    }

    /// A directory whose outputs and inputs can be made to fail on `close()`.
    struct FailingCloseDirectory {
        inner: Arc<dyn Directory>,
        fail: AtomicBool,
        fail_input: AtomicBool,
    }

    impl FailingCloseDirectory {
        fn new(inner: Arc<dyn Directory>) -> Arc<Self> {
            Arc::new(Self {
                inner,
                fail: AtomicBool::new(false),
                fail_input: AtomicBool::new(false),
            })
        }

        fn set_failing(&self, failing: bool) {
            self.fail.store(failing, AtomicOrdering::SeqCst);
        }

        fn set_input_failing(&self, failing: bool) {
            self.fail_input.store(failing, AtomicOrdering::SeqCst);
        }
    }

    impl Directory for FailingCloseDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            let out = self.inner.create_output(name, context)?;
            if self.fail.load(AtomicOrdering::SeqCst) {
                Ok(Box::new(FailingCloseOutput { inner: out }))
            } else {
                Ok(out)
            }
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexInput>> {
            let input = self.inner.open_input(name, context)?;
            if self.fail_input.load(AtomicOrdering::SeqCst) {
                Ok(Box::new(FailingCloseInput { inner: input }))
            } else {
                Ok(input)
            }
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn get_pending_deletions(&self) -> Result<HashSet<String>> {
            self.inner.get_pending_deletions()
        }
    }

    #[test]
    fn persistent_snapshot_reports_a_save_file_that_fails_to_close() {
        let backing = ram_directory();
        let dir = FailingCloseDirectory::new(Arc::clone(&backing));

        // Produce a perfectly valid save file first.
        {
            let policy = PersistentSnapshotDeletionPolicy::new(
                Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
                Arc::clone(&dir) as Arc<dyn Directory>,
            )
            .unwrap();
            policy.on_init(&commits(&backing, &[5])).unwrap();
            policy.snapshot().unwrap();
        }
        assert_eq!(snapshot_files(backing.as_ref()), vec!["snapshots_0"]);

        // Reading it back now fails on `close()`. Java's `finally` clause sits
        // outside the `catch (IOException)`, so this is *not* the swallowed
        // "unparseable content" case: it must be reported
        // (`PersistentSnapshotDeletionPolicy.java:240-242`).
        dir.set_input_failing(true);
        let err = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir) as Arc<dyn Directory>,
            OpenMode::CREATE_OR_APPEND,
        )
        .expect_err("a save file that cannot be closed must not be silently ignored");
        assert!(
            matches!(&err, LuceneError::Io(io) if io.to_string().contains("could not be closed")),
            "unexpected error: {err:?}"
        );

        // The file itself is untouched, so a healthy reopen still works.
        dir.set_input_failing(false);
        let policy = PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir) as Arc<dyn Directory>,
            OpenMode::APPEND,
        )
        .unwrap();
        assert_eq!(policy.get_snapshot_count(), 1);
    }

    #[test]
    fn persistent_snapshot_shares_one_monitor_with_the_policy_it_wraps() {
        let dir = ram_directory();
        let entered = Gate::new();
        let proceed = Gate::new();
        let policy = Arc::new(
            PersistentSnapshotDeletionPolicy::new(
                Arc::new(BlockingPrimary {
                    inner: KeepOnlyLastCommitDeletionPolicy::new(),
                    phase: Phase::OnCommit,
                    entered: Arc::clone(&entered),
                    proceed: Arc::clone(&proceed),
                }),
                Arc::clone(&dir),
            )
            .unwrap(),
        );

        policy.on_init(&commits(&dir, &[1])).unwrap();

        let live = commits(&dir, &[1, 2]);
        let committer = {
            let policy = Arc::clone(&policy);
            let live = live.clone();
            std::thread::spawn(move || policy.on_commit(&live))
        };
        entered.wait();
        assert!(live[0].is_deleted());

        // Java inherits the `synchronized` methods, so the persistent policy and
        // the policy it extends share *one* monitor. Rucene composes instead of
        // inheriting, so this pins the delegation: if the persistent policy grew
        // a monitor of its own, `snapshot()` would slip through here.
        let done = Gate::new();
        let outcome: SnapshotOutcome = Arc::new(Mutex::new(None));
        let started = Gate::new();
        let snapshotter = {
            let policy = Arc::clone(&policy);
            let started = Arc::clone(&started);
            let done = Arc::clone(&done);
            let outcome = Arc::clone(&outcome);
            std::thread::spawn(move || {
                started.open();
                let result = policy
                    .snapshot()
                    .map(|commit| (commit.get_generation(), commit.is_deleted()));
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                done.open();
            })
        };
        started.wait();
        let ran_during_on_commit = done.wait_timeout(CONTENTION_WINDOW);

        proceed.open();
        committer.join().unwrap().unwrap();
        snapshotter.join().unwrap();
        let (generation, deleted) = outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap()
            .expect("snapshot must succeed");

        assert!(
            !ran_during_on_commit,
            "snapshot() ran while on_commit was in flight; both must share one monitor"
        );
        assert_eq!(generation, 2);
        assert!(!deleted);
        assert_eq!(policy.get_snapshot_count(), 1);
        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_0"));
    }

    #[test]
    fn persistent_snapshot_recovers_when_the_save_file_fails_to_close() {
        let backing = ram_directory();
        let dir = FailingCloseDirectory::new(Arc::clone(&backing));
        let policy = PersistentSnapshotDeletionPolicy::new(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            Arc::clone(&dir) as Arc<dyn Directory>,
        )
        .unwrap();

        let initial = commits(&backing, &[7]);
        policy.on_init(&initial).unwrap();

        // The save file cannot be flushed: `persist()` must fail...
        dir.set_failing(true);
        let err = policy
            .snapshot()
            .expect_err("snapshot must report the I/O failure");
        assert!(
            matches!(&err, LuceneError::Io(io) if io.to_string().contains("no space left")),
            "unexpected error: {err:?}"
        );

        // ... the in-memory state must be rolled back, ...
        assert_eq!(policy.get_snapshot_count(), 0);
        assert_eq!(policy.get_last_save_file(), None);

        // ... and the unusable save file must not be left behind, so the next
        // attempt can reuse the same generation. This is the deliberate
        // divergence documented on `persist()`: Lucene 10.5.0 keeps the file
        // (PersistentSnapshotDeletionPolicy.java:174-186), so its next
        // `snapshot()` fails a second time with `FileAlreadyExistsException`
        // before the generation frees up again.
        assert_eq!(snapshot_files(backing.as_ref()), Vec::<String>::new());

        dir.set_failing(false);
        let pinned = policy.snapshot().expect("the policy must still be usable");
        assert_eq!(pinned.get_generation(), 7);
        assert_eq!(policy.get_snapshot_count(), 1);
        assert_eq!(policy.get_last_save_file().as_deref(), Some("snapshots_0"));
        assert_eq!(snapshot_files(backing.as_ref()), vec!["snapshots_0"]);
    }

    #[test]
    fn persistent_snapshot_release_of_an_unknown_generation_does_not_persist() {
        let dir = ram_directory();
        let policy = persistent_policy(&dir);
        policy.on_init(&commits(&dir, &[1])).unwrap();
        policy.snapshot().unwrap();
        let before = snapshot_files(dir.as_ref());

        let err = policy.release_gen(99).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)), "{err}");
        assert_eq!(snapshot_files(dir.as_ref()), before);
        assert_eq!(policy.get_snapshot_count(), 1);
    }
}
