//! Tracking and deletion of the index files a directory still needs.
//!
//! Port of `org.apache.lucene.index.IndexFileDeleter` from Apache Lucene Core
//! 10.5.0 (`lucene/core/src/java/org/apache/lucene/index/IndexFileDeleter.java`).
//!
//! This type keeps track of every `SegmentInfos` that is still *live* — either
//! because a `segments_N` file in the directory names it (a commit), or because
//! a writer is actively updating it in memory and has not committed it yet. It
//! reference-counts those `SegmentInfos` down to individual file names, and
//! deletes a file once the last commit referencing it is gone.
//!
//! The division of labour matches Lucene's:
//!
//! - the [`IndexDeletionPolicy`] decides **when a commit point may go**;
//! - `IndexFileDeleter` decides **which files that makes unreferenced**;
//! - [`FileDeleter`] performs the deletion and the ordering that makes it safe.
//!
//! # Deferred deletion lives in the `Directory`, not here
//!
//! A delete that fails — the case Windows forced on Lucene, where a file still
//! open cannot be unlinked — must be retried later rather than be fatal. In
//! Lucene 10.5.0 that retry is **not** in this class. Verified against the
//! 10.5.0 sources: `IndexFileDeleter.java` contains no `deletable` list and no
//! `deletePendingFiles`; the pending set and its retry live in the directory,
//! at `FSDirectory.java:282,315,335,407` (`deletePendingFiles()` and
//! `getPendingDeletions()`). `IndexFileDeleter` merely *consults*
//! `Directory::get_pending_deletions` once during construction, to fold those
//! names into the set fed to [`inflate_gens`] (`IndexFileDeleter.java:211-217`).
//!
//! Rucene follows 10.5.0: the retry lives in `FSDirectory::delete_pending_files`
//! (`src/store.rs`), which is already ported, and this module does not carry a
//! `deletable` list. Adding one here would be a divergence from the reference
//! version, not fidelity to it.
//!
//! # The `IndexWriter` seam
//!
//! Java's constructor takes a non-null `IndexWriter` and uses it for exactly
//! three things: an assertion that the caller holds the writer's monitor
//! (`locked()`, `IndexFileDeleter.java:94-96`), one info-stream message built
//! from `writer.segString(...)` (`IndexFileDeleter.java:561`), and
//! `ensureOpen()`/`isClosed()`, which consult the writer's closed state and
//! tragic exception (`IndexFileDeleter.java:382-403`).
//!
//! `IndexWriter` is not ported yet, so this type carries no writer. The two
//! consequences are recorded honestly rather than papered over:
//!
//! - The `locked()` assertion has no analogue and needs none: Java asserts by
//!   convention what Rust enforces in the type system, since every mutating
//!   method here takes `&mut self`.
//! - `ensure_open()` and `is_closed()` are **not** provided. They are pure
//!   functions of `IndexWriter` state and cannot be written before it exists.
//!   They must be added when `IndexWriter` lands; nothing else in this class
//!   depends on them (in Lucene, `ensureOpen` is reachable only through
//!   `isClosed`, which only `IndexWriter.isDeleterClosed()` calls).
//!
//! Neither affects reference counting or deletion, so the file-lifecycle
//! behaviour ported here is complete.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::error::{LuceneError, Result};
use crate::index::index_commit::IndexCommit;
use crate::index::index_deletion_policy::IndexDeletionPolicy;
use crate::index::index_file_names::{
    is_codec_file, parse_generation, parse_segment_name, PENDING_SEGMENTS, SEGMENTS,
};
use crate::index::segment_infos::SegmentInfos;
use crate::store::Directory;
use crate::util::file_deleter::{FileDeleter, Messenger, MsgType};
use crate::util::InfoStream;

/// The info-stream component name Lucene logs index-file deletion under.
const IFD: &str = "IFD";

/// The name of the write lock, which is never reference-counted.
///
/// Equivalent to `IndexWriter.WRITE_LOCK_NAME`. Defined here because
/// `IndexWriter` is not ported yet; it moves to `IndexWriter` when that lands.
pub const WRITE_LOCK_NAME: &str = "write.lock";

/// Radix Lucene encodes segment names and generations in.
///
/// Java spells this `Character.MAX_RADIX`.
const MAX_RADIX: u32 = 36;

/// A commit point held by an [`IndexFileDeleter`], and handed to the deletion
/// policy.
///
/// Port of the private inner class `IndexFileDeleter.CommitPoint`.
///
/// Note, as Lucene notes, that the natural ordering of commit points (by
/// generation) is inconsistent with equality.
pub struct CommitPoint {
    files: HashSet<String>,
    segments_file_name: String,
    deleted: AtomicBool,
    directory_orig: Arc<dyn Directory>,
    commits_to_delete: Arc<Mutex<Vec<Arc<CommitPoint>>>>,
    generation: i64,
    user_data: HashMap<String, String>,
    segment_count: i32,
    /// Self-reference, so that [`CommitPoint::delete`] can enqueue *this*
    /// commit point on `commits_to_delete`.
    ///
    /// Java's inner class simply writes `commitsToDelete.add(this)`
    /// (`IndexFileDeleter.java:714`). A Rust `&self` cannot recover its own
    /// `Arc`, so the `Arc` is built with [`Arc::new_cyclic`] and the weak
    /// handle stored. This is a mechanical adaptation of the same structure,
    /// not a behavioural change.
    self_ref: Weak<CommitPoint>,
}

impl std::fmt::Debug for CommitPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Matches Java's `IndexFileDeleter.CommitPoint(segments_N)` toString.
        write!(
            f,
            "IndexFileDeleter.CommitPoint({})",
            self.segments_file_name
        )
    }
}

impl CommitPoint {
    /// Builds a commit point describing `segment_infos`.
    ///
    /// Equivalent to the `CommitPoint(Collection<CommitPoint>, Directory,
    /// SegmentInfos)` constructor.
    fn new(
        commits_to_delete: Arc<Mutex<Vec<Arc<CommitPoint>>>>,
        directory_orig: Arc<dyn Directory>,
        segment_infos: &SegmentInfos,
    ) -> Result<Arc<Self>> {
        let user_data = segment_infos.user_data().clone();
        let segments_file_name = segment_infos.segments_file_name().ok_or_else(|| {
            LuceneError::IllegalState(
                "cannot build a commit point from SegmentInfos with no segments file name"
                    .to_string(),
            )
        })?;
        let generation = segment_infos.generation();
        let files = segment_infos.files(true)?;
        let segment_count = segment_infos.size() as i32;

        Ok(Arc::new_cyclic(|self_ref| CommitPoint {
            files,
            segments_file_name,
            deleted: AtomicBool::new(false),
            directory_orig,
            commits_to_delete,
            generation,
            user_data,
            segment_count,
            self_ref: self_ref.clone(),
        }))
    }

    /// Returns the files this commit point references, including its
    /// `segments_N`.
    fn files(&self) -> &HashSet<String> {
        &self.files
    }
}

impl IndexCommit for CommitPoint {
    fn get_segments_file_name(&self) -> String {
        self.segments_file_name.clone()
    }

    fn get_file_names(&self) -> Result<HashSet<String>> {
        Ok(self.files.clone())
    }

    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory_orig)
    }

    /// Marks this commit point for removal.
    ///
    /// Called only by the deletion policy. The commit point is queued on the
    /// deleter's `commits_to_delete` list; the files are not decremented until
    /// the deleter next runs `delete_commits`.
    ///
    /// Equivalent to `CommitPoint.delete()`.
    fn delete(&self) -> Result<()> {
        // `swap` gives Java's `if (!deleted) { deleted = true; ... }` in one
        // step, so a commit point is enqueued at most once even under races.
        if !self.deleted.swap(true, Ordering::AcqRel) {
            if let Some(this) = self.self_ref.upgrade() {
                self.commits_to_delete
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(this);
            }
        }
        Ok(())
    }

    fn is_deleted(&self) -> bool {
        self.deleted.load(Ordering::Acquire)
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

/// Tracks which index files are still referenced and deletes the rest.
///
/// Port of `org.apache.lucene.index.IndexFileDeleter`.
///
/// # Thread safety
///
/// As in Lucene, this type is not internally synchronised — Java guards it with
/// `synchronized (writer)` and asserts so in `locked()`. Rucene expresses the
/// same contract through `&mut self`, so exclusive access is checked at compile
/// time. The one exception is [`CommitPoint::delete`], which the deletion policy
/// may call re-entrantly while the deleter is mid-`checkpoint`; that path uses
/// interior mutability, exactly as Java's does.
pub struct IndexFileDeleter {
    /// Every commit (`segments_N`) currently in the index, oldest to newest.
    ///
    /// With the default `KeepOnlyLastCommitDeletionPolicy` this holds a single
    /// entry; policies that retain history make it longer.
    commits: Vec<Arc<CommitPoint>>,

    /// Files incremented by the previous *non-commit* checkpoint.
    last_files: Vec<String>,

    /// Commits the deletion policy has asked to remove, not yet processed.
    commits_to_delete: Arc<Mutex<Vec<Arc<CommitPoint>>>>,

    info_stream: Arc<dyn InfoStream>,

    /// The directory commit-point metadata is read from and reported against.
    directory_orig: Arc<dyn Directory>,

    /// The directory files are actually deleted from.
    directory: Arc<dyn Directory>,

    policy: Arc<dyn IndexDeletionPolicy>,

    /// Whether the commit the deleter was opened on was deleted during `on_init`.
    ///
    /// Equivalent to the package-private field `startingCommitDeleted`.
    pub starting_commit_deleted: bool,

    /// The newest `SegmentInfos` seen while scanning the directory.
    last_segment_infos: Option<SegmentInfos>,

    file_deleter: FileDeleter,
}

impl std::fmt::Debug for IndexFileDeleter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexFileDeleter")
            .field("commits", &self.commits.len())
            .field("last_files", &self.last_files.len())
            .field("starting_commit_deleted", &self.starting_commit_deleted)
            .field("file_deleter", &self.file_deleter)
            .finish()
    }
}

impl IndexFileDeleter {
    /// Initialises the deleter: finds every previous commit in the directory,
    /// increments the files each references, and lets the policy delete commits.
    /// Any file not referenced by a surviving commit is removed.
    ///
    /// `files` is the directory listing to adopt. `segment_infos` is the
    /// in-memory commit the caller is working from; it is mutated by
    /// [`inflate_gens`] so that future writes never collide with a generation
    /// already present on disk.
    ///
    /// The caller must hold the index write lock: this opens `segments_N` files
    /// directly, with no retry logic.
    ///
    /// Equivalent to the `IndexFileDeleter(...)` constructor, minus the
    /// `IndexWriter` argument (see the module docs on the `IndexWriter` seam).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        files: &[String],
        directory_orig: Arc<dyn Directory>,
        directory: Arc<dyn Directory>,
        policy: Arc<dyn IndexDeletionPolicy>,
        segment_infos: &mut SegmentInfos,
        info_stream: Arc<dyn InfoStream>,
        initial_index_exists: bool,
        is_reader_init: bool,
    ) -> Result<Self> {
        let current_segments_file = segment_infos.segments_file_name();

        if info_stream.is_enabled(IFD) {
            info_stream.message(
                IFD,
                &format!(
                    "init: current segments file is \"{}\"; deletionPolicy={policy:?}",
                    current_segments_file.as_deref().unwrap_or("null")
                ),
            );
        }

        let commits_to_delete: Arc<Mutex<Vec<Arc<CommitPoint>>>> = Arc::new(Mutex::new(Vec::new()));

        let messenger: Messenger = {
            let info_stream = Arc::clone(&info_stream);
            Arc::new(move |msg_type, msg| {
                // Ref-count chatter is dropped; file events go to the stream.
                // Equivalent to `IndexFileDeleter.logInfo`.
                if msg_type != MsgType::Ref && info_stream.is_enabled(IFD) {
                    info_stream.message(IFD, msg);
                }
            })
        };
        let mut file_deleter = FileDeleter::new(Arc::clone(&directory), Some(messenger));

        // First pass: walk the directory listing and seed the reference counts.
        let mut commits: Vec<Arc<CommitPoint>> = Vec::new();
        let mut current_commit_point: Option<Arc<CommitPoint>> = None;
        let mut last_segment_infos: Option<SegmentInfos> = None;

        if current_segments_file.is_some() {
            for file_name in files {
                if file_name.ends_with(WRITE_LOCK_NAME) {
                    continue;
                }
                if !(is_codec_file(file_name)
                    || file_name.starts_with(SEGMENTS)
                    || file_name.starts_with(PENDING_SEGMENTS))
                {
                    continue;
                }

                // Adopt the file with an initial count of zero.
                file_deleter.init_ref_count(file_name);

                if file_name.starts_with(SEGMENTS) {
                    // A commit point. Load it and increment everything it
                    // references, its own `segments_N` included.
                    if info_stream.is_enabled(IFD) {
                        info_stream.message(IFD, &format!("init: load commit \"{file_name}\""));
                    }
                    let sis = SegmentInfos::read_commit(directory_orig.as_ref(), file_name)?;

                    let commit_point = CommitPoint::new(
                        Arc::clone(&commits_to_delete),
                        Arc::clone(&directory_orig),
                        &sis,
                    )?;
                    if sis.generation() == segment_infos.generation() {
                        current_commit_point = Some(Arc::clone(&commit_point));
                    }
                    commits.push(commit_point);
                    for file in sis.files(true)? {
                        file_deleter.inc_ref(&file);
                    }

                    if last_segment_infos
                        .as_ref()
                        .map_or(true, |last| sis.generation() > last.generation())
                    {
                        last_segment_infos = Some(sis);
                    }
                }
            }
        }

        if current_commit_point.is_none() && current_segments_file.is_some() && initial_index_exists
        {
            // We never saw the `segments_N` matching the incoming SegmentInfos,
            // yet it must exist because the caller holds the write lock. A stale
            // directory listing can do this (e.g. an NFS client caching it), so
            // open the commit point explicitly.
            let name = current_segments_file
                .as_deref()
                .expect("checked is_some above");
            let sis = SegmentInfos::read_commit(directory_orig.as_ref(), name).map_err(|e| {
                LuceneError::CorruptIndex(format!(
                    "unable to read current segments_N file \"{name}\": {e}"
                ))
            })?;
            if info_stream.is_enabled(IFD) {
                info_stream.message(IFD, &format!("forced open of current segments file {name}"));
            }
            let commit_point = CommitPoint::new(
                Arc::clone(&commits_to_delete),
                Arc::clone(&directory_orig),
                &sis,
            )?;
            current_commit_point = Some(Arc::clone(&commit_point));
            commits.push(commit_point);
            for file in sis.files(true)? {
                file_deleter.inc_ref(&file);
            }
        }

        let mut this = Self {
            commits,
            last_files: Vec::new(),
            commits_to_delete,
            info_stream: Arc::clone(&info_stream),
            directory_orig: Arc::clone(&directory_orig),
            directory,
            policy,
            starting_commit_deleted: false,
            last_segment_infos,
            file_deleter,
        };

        if is_reader_init {
            // The incoming SegmentInfos may hold NRT changes that no commit
            // reflects yet, so its files need protecting too.
            this.checkpoint(segment_infos, false)?;
        }

        // Keep the commit list sorted oldest to newest. Java uses
        // `CollectionUtil.timSort`, relying on `IndexCommit.compareTo`, which
        // compares generations (`IndexCommit.java:101-110`).
        this.commits.sort_by_key(|c| c.generation);

        let mut relevant_files: HashSet<String> = this
            .file_deleter
            .get_all_files()
            .map(str::to_string)
            .collect();
        relevant_files.extend(this.directory_orig.get_pending_deletions()?);

        // Note the reference counts hold only "normal" file names — never
        // `write.lock`.
        inflate_gens(segment_infos, &relevant_files, info_stream.as_ref())?;

        // Anything still at a count of zero is abandoned — typically the debris
        // of an IndexWriter that crashed.
        let to_delete = this.file_deleter.get_unrefed_files();
        for file_name in &to_delete {
            if file_name.starts_with(SEGMENTS) {
                return Err(LuceneError::IllegalState(format!(
                    "file \"{file_name}\" has refCount=0, which should never happen on init"
                )));
            }
            if info_stream.is_enabled(IFD) {
                info_stream.message(
                    IFD,
                    &format!("init: removing unreferenced file \"{file_name}\""),
                );
            }
        }
        this.file_deleter.delete_files_if_no_ref(&to_delete)?;

        // Finally let the policy drop commits on startup.
        this.policy.on_init(&this.commits_as_index_commits())?;

        // Always protect the incoming SegmentInfos: it is not always the most
        // recent commit.
        this.checkpoint(segment_infos, false)?;

        this.starting_commit_deleted = current_commit_point
            .as_ref()
            .is_some_and(|c| c.is_deleted());

        this.delete_commits()?;

        Ok(this)
    }

    /// Returns the live commits as trait objects, for handing to the policy.
    fn commits_as_index_commits(&self) -> Vec<Arc<dyn IndexCommit>> {
        self.commits
            .iter()
            .map(|c| Arc::clone(c) as Arc<dyn IndexCommit>)
            .collect()
    }

    /// Returns the newest `SegmentInfos` found while scanning the directory.
    ///
    /// **Addition, not a port.** Lucene keeps `lastSegmentInfos` in a `private`
    /// field with no accessor. Exposed here for observation only; nothing in the
    /// port reads it, and it changes no behaviour.
    pub fn last_segment_infos(&self) -> Option<&SegmentInfos> {
        self.last_segment_infos.as_ref()
    }

    /// Returns the commits currently known to the deleter, oldest to newest.
    ///
    /// **Addition, not a port.** Lucene keeps `commits` in a `private` field and
    /// lets the deletion policy see it only as the argument to `onInit`/
    /// `onCommit`. Exposed here so the portability tests can check that Rucene
    /// rebuilt the same commit points Lucene did; it hands out the same
    /// `Arc`s the policy receives and changes no behaviour.
    pub fn commits(&self) -> Vec<Arc<dyn IndexCommit>> {
        self.commits_as_index_commits()
    }

    /// Removes the commit points the policy queued, decrementing their files.
    ///
    /// Equivalent to `IndexFileDeleter.deleteCommits`.
    fn delete_commits(&mut self) -> Result<()> {
        let queued: Vec<Arc<CommitPoint>> = {
            let mut guard = self
                .commits_to_delete
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };

        if queued.is_empty() {
            return Ok(());
        }

        // Decrement everything the now-dead commits referenced. Every commit is
        // processed even if one fails; the first error is the one reported.
        let mut first_error: Option<LuceneError> = None;
        for commit in &queued {
            if self.info_stream.is_enabled(IFD) {
                self.info_stream.message(
                    IFD,
                    &format!(
                        "deleteCommits: now decRef commit \"{}\"",
                        commit.segments_file_name
                    ),
                );
            }
            if let Err(e) = self.file_deleter.dec_ref_files(commit.files()) {
                first_error.get_or_insert(e);
            }
        }

        // Compact the commit list, preserving order.
        self.commits.retain(|c| !c.is_deleted());

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Re-lists the directory and removes files nothing references.
    ///
    /// The writer calls this after an error forced a rollback, since that may
    /// have left unreferenced files behind.
    ///
    /// Equivalent to `IndexFileDeleter.refresh`.
    pub fn refresh(&mut self) -> Result<()> {
        let mut to_delete: HashSet<String> = HashSet::new();

        for file_name in self.directory.list_all()? {
            if file_name.ends_with(WRITE_LOCK_NAME) || self.file_deleter.exists(&file_name) {
                continue;
            }
            // `pending_segments_N` is cleared only here, during rollback: it is
            // never reference-counted. Anything left over is deleted on the next
            // writer start-up anyway.
            if is_codec_file(&file_name)
                || file_name.starts_with(SEGMENTS)
                || file_name.starts_with(PENDING_SEGMENTS)
            {
                if self.info_stream.is_enabled(IFD) {
                    self.info_stream.message(
                        IFD,
                        &format!(
                            "refresh: removing newly created unreferenced file \"{file_name}\""
                        ),
                    );
                }
                to_delete.insert(file_name);
            }
        }

        self.file_deleter.delete_files_if_no_ref(&to_delete)
    }

    /// Decrements the files held by the last non-commit checkpoint.
    ///
    /// Equivalent to `IndexFileDeleter.close`. Java's class implements
    /// `Closeable`; here it is an inherent fallible method, the same shape
    /// `Directory::close` already uses in this crate, because the release can
    /// fail and the failure must reach the caller rather than be swallowed by a
    /// `Drop`.
    pub fn close(&mut self) -> Result<()> {
        if self.last_files.is_empty() {
            return Ok(());
        }
        let files = std::mem::take(&mut self.last_files);
        self.file_deleter.dec_ref_files(files)
    }

    /// Re-runs [`IndexDeletionPolicy::on_commit`] over the known commits.
    ///
    /// Useful with a policy that holds commits open: the application may know
    /// that some are no longer held and want the now-unused ones collected.
    ///
    /// Equivalent to `IndexFileDeleter.revisitPolicy`.
    pub fn revisit_policy(&mut self) -> Result<()> {
        if self.info_stream.is_enabled(IFD) {
            self.info_stream.message(IFD, "now revisitPolicy");
        }

        if !self.commits.is_empty() {
            debug_assert!(
                self.commits.iter().all(|c| !c.is_deleted()),
                "a commit in the live list was already deleted"
            );
            self.policy.on_commit(&self.commits_as_index_commits())?;
            self.delete_commits()?;
        }
        Ok(())
    }

    /// Records that the index has reached a consistent state.
    ///
    /// The writer calls this once new files are on disk and the in-memory
    /// `SegmentInfos` points at them. This may or may not be a commit —
    /// `segments_N` may or may not have been written.
    ///
    /// The files the new `SegmentInfos` references are incremented and those
    /// seen at the previous checkpoint are decremented. When `is_commit` is
    /// true the deletion policy is consulted and any commits it drops have their
    /// files decremented as well.
    ///
    /// Equivalent to `IndexFileDeleter.checkpoint`.
    pub fn checkpoint(&mut self, segment_infos: &SegmentInfos, is_commit: bool) -> Result<()> {
        if self.info_stream.is_enabled(IFD) {
            // Java also logs `writer.segString(...)` here; that part needs
            // `IndexWriter`, which is not ported yet (see the module docs).
            self.info_stream.message(
                IFD,
                &format!(
                    "now checkpoint [{} segments; isCommit = {is_commit}]",
                    segment_infos.size()
                ),
            );
        }

        self.inc_ref_segment_infos(segment_infos, is_commit)?;

        if is_commit {
            let commit_point = CommitPoint::new(
                Arc::clone(&self.commits_to_delete),
                Arc::clone(&self.directory_orig),
                segment_infos,
            )?;
            self.commits.push(commit_point);

            debug_assert!(
                self.commits.iter().all(|c| !c.is_deleted()),
                "a commit in the live list was already deleted"
            );
            self.policy.on_commit(&self.commits_as_index_commits())?;

            // Decrement the files of whatever the policy dropped.
            self.delete_commits()?;
        } else {
            // Decrement the previous checkpoint's files, then remember this
            // checkpoint's for the next one. `last_files` is cleared even if the
            // decrement fails, matching Java's try/finally.
            let previous = std::mem::take(&mut self.last_files);
            let result = self.file_deleter.dec_ref_files(previous);
            self.last_files.extend(segment_infos.files(false)?);
            result?;
        }

        Ok(())
    }

    /// Increments the files referenced by `segment_infos`.
    ///
    /// When `is_commit` is true the `segments_N` file itself is included.
    ///
    /// Equivalent to `IndexFileDeleter.incRef(SegmentInfos, boolean)`.
    pub fn inc_ref_segment_infos(
        &mut self,
        segment_infos: &SegmentInfos,
        is_commit: bool,
    ) -> Result<()> {
        for file_name in segment_infos.files(is_commit)? {
            self.file_deleter.inc_ref(&file_name);
        }
        Ok(())
    }

    /// Increments the reference count of every named file.
    ///
    /// This is how a merge or an NRT reader pins the files it is using: the
    /// writer increments the reader's segment files and decrements them when the
    /// reader is dropped (see `IndexWriter.java:4070,5202,5274` in Lucene
    /// 10.5.0).
    ///
    /// Equivalent to `IndexFileDeleter.incRef(Collection<String>)`.
    pub fn inc_ref_files<I, S>(&mut self, files: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.file_deleter.inc_ref_files(files);
    }

    /// Decrements every named file, deleting those that reach zero.
    ///
    /// Every file is decremented even if one fails; the first error is returned.
    ///
    /// Equivalent to `IndexFileDeleter.decRef(Collection<String>)`.
    pub fn dec_ref_files<I, S>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.file_deleter.dec_ref_files(files)
    }

    /// Decrements the files referenced by `segment_infos`, excluding its
    /// `segments_N`.
    ///
    /// Equivalent to `IndexFileDeleter.decRef(SegmentInfos)`.
    pub fn dec_ref_segment_infos(&mut self, segment_infos: &SegmentInfos) -> Result<()> {
        let files = segment_infos.files(false)?;
        self.file_deleter.dec_ref_files(files)
    }

    /// Returns whether `file_name` is tracked and currently referenced.
    ///
    /// Equivalent to `IndexFileDeleter.exists`.
    pub fn exists(&self, file_name: &str) -> bool {
        self.file_deleter.exists(file_name)
    }

    /// Returns the current reference count of `file_name`, or `0`.
    ///
    /// **Addition, not a port, and test-only.** Lucene's `IndexFileDeleter`
    /// holds its `FileDeleter` in a `private` field and exposes no such
    /// accessor. This one exists solely so the unit tests can assert reference
    /// counts, and it is compiled out of every non-test build, so the shipped
    /// API surface stays exactly Lucene's.
    #[cfg(test)]
    pub(crate) fn get_ref_count(&self, file_name: &str) -> i32 {
        self.file_deleter.get_ref_count(file_name)
    }

    /// Deletes the named files, but only those that were never incremented.
    ///
    /// Equivalent to `IndexFileDeleter.deleteNewFiles`.
    pub fn delete_new_files<I, S>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.file_deleter.delete_files_if_no_ref(files)
    }
}

/// Pushes every generation past what the directory already shows, so that a
/// writer which did not close or roll back gracefully — a crash, a power cut —
/// can never write a file name that already exists.
///
/// `files` is the set of names to learn from: the reference-counted names plus
/// whatever the directory reports as pending deletion.
///
/// Equivalent to the static `IndexFileDeleter.inflateGens`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] if a per-segment file name carries a
/// segment ordinal that does not fit in an `i64`. Java lets the resulting
/// `NumberFormatException` escape at the same point
/// (`IndexFileDeleter.java:303-304` is deliberately *not* inside a `try`, unlike
/// the `parseGeneration` call below it); Rucene reports it as an error rather
/// than panicking, so that a corrupt or hostile directory listing cannot abort
/// the process.
pub fn inflate_gens(
    infos: &mut SegmentInfos,
    files: &HashSet<String>,
    info_stream: &dyn InfoStream,
) -> Result<()> {
    let mut max_segment_gen = i64::MIN;
    let mut max_segment_name = i64::MIN;

    // Confusingly this is the union of the live-docs, field-infos and doc-values
    // generations. Lucene accepts the imprecision — doc-values updates will jump
    // to the generation after live docs', for instance — because there is no API
    // to ask the codec which file is which.
    let mut max_per_segment_gen: HashMap<String, i64> = HashMap::new();

    for file_name in files {
        if file_name == WRITE_LOCK_NAME {
            // Never carries a generation.
        } else if let Some(rest) = file_name.strip_prefix(PENDING_SEGMENTS) {
            // `pending_segments_N` -> read the `_N` that follows.
            // Checked before the `segments` arm: `PENDING_SEGMENTS` does not
            // start with `SEGMENTS`, but testing it first keeps the two
            // unambiguous regardless of future constant changes.
            if let Ok(gen) =
                SegmentInfos::generation_from_segments_file_name(&format!("{SEGMENTS}{rest}"))
            {
                max_segment_gen = max_segment_gen.max(gen);
            }
            // A trash file: anything starting with `pending_segments` is allowed
            // here, so an unparsable tail is ignored rather than fatal.
        } else if file_name.starts_with(SEGMENTS) {
            if let Ok(gen) = SegmentInfos::generation_from_segments_file_name(file_name) {
                max_segment_gen = max_segment_gen.max(gen);
            }
            // Likewise trash-tolerant.
        } else {
            let segment_name = parse_segment_name(file_name);
            debug_assert!(
                segment_name.starts_with('_'),
                "unexpected file name: {file_name}"
            );

            if file_name.to_lowercase().ends_with(".tmp") {
                // A temp file: it carries no meaningful generation.
                continue;
            }

            // Java writes `segmentName.substring(1)`. Drop one *character*, not
            // one byte: a pending-deletion name is arbitrary and may begin with
            // a multi-byte UTF-8 character, which byte-slicing would panic on.
            let ordinal_str = {
                let mut chars = segment_name.chars();
                chars.next();
                chars.as_str()
            };
            let ordinal = i64::from_str_radix(ordinal_str, MAX_RADIX).map_err(|e| {
                LuceneError::CorruptIndex(format!(
                    "cannot parse segment ordinal from \"{file_name}\": {e}"
                ))
            })?;
            max_segment_name = max_segment_name.max(ordinal);

            let entry = max_per_segment_gen
                .entry(segment_name.to_string())
                .or_insert(0);
            if let Ok(gen) = parse_generation(file_name) {
                *entry = (*entry).max(gen);
            }
            // Unparsable generation: the codec file-name pattern is only so
            // good, so trash is tolerated here too.
        }
    }

    // The generation is advanced before a write, so setting it to the maximum
    // seen is enough to guarantee the next name is fresh.
    infos.set_next_write_generation(infos.generation().max(max_segment_gen))?;

    // `i64::MIN + 1` is representable, so this cannot overflow even when no
    // per-segment file was seen; the comparison then simply fails.
    let next_counter = max_segment_name.wrapping_add(1);
    if infos.counter < next_counter {
        if info_stream.is_enabled(IFD) {
            info_stream.message(
                IFD,
                &format!(
                    "init: inflate infos.counter to {next_counter} vs current={}",
                    infos.counter
                ),
            );
        }
        infos.counter = next_counter;
    }

    for info in infos.iter_mut() {
        let Some(&gen) = max_per_segment_gen.get(&info.info.name) else {
            // Java asserts this is non-null. A segment named by the commit but
            // with no file on disk means the directory listing and the commit
            // disagree, which is a corrupt index rather than a bug here.
            return Err(LuceneError::CorruptIndex(format!(
                "segment \"{}\" is referenced by the commit but has no files in the directory",
                info.info.name
            )));
        };
        let next = gen + 1;

        if info.get_next_del_gen() < next {
            if info_stream.is_enabled(IFD) {
                info_stream.message(
                    IFD,
                    &format!(
                        "init: seg={} set nextWriteDelGen={next} vs current={}",
                        info.info.name,
                        info.get_next_del_gen()
                    ),
                );
            }
            info.set_next_write_del_gen(next);
        }
        if info.get_next_field_infos_gen() < next {
            if info_stream.is_enabled(IFD) {
                info_stream.message(
                    IFD,
                    &format!(
                        "init: seg={} set nextWriteFieldInfosGen={next} vs current={}",
                        info.info.name,
                        info.get_next_field_infos_gen()
                    ),
                );
            }
            info.set_next_write_field_infos_gen(next);
        }
        if info.get_next_doc_values_gen() < next {
            if info_stream.is_enabled(IFD) {
                info_stream.message(
                    IFD,
                    &format!(
                        "init: seg={} set nextWriteDocValuesGen={next} vs current={}",
                        info.info.name,
                        info.get_next_doc_values_gen()
                    ),
                );
            }
            info.set_next_write_doc_values_gen(next);
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::lucene99::Lucene99SegmentInfoFormat;
    use crate::codecs::tests::{test_segment_info, DummyCodec};
    use crate::codecs::{register_codec, Codec, FilterCodec, SegmentInfoFormat};
    use crate::index::index_deletion_policy::{
        KeepOnlyLastCommitDeletionPolicy, NoDeletionPolicy, SnapshotDeletionPolicy,
    };
    use crate::index::SegmentCommitInfo;
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::string_helper::StringHelper;
    use crate::util::NoOutputInfoStream;

    fn codec() -> Arc<dyn Codec> {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        REGISTER.call_once(|| {
            let registered = FilterCodec::new("IFDTestCodec", Arc::new(DummyCodec::new("Dummy")))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new());
            let _ = register_codec("IFDTestCodec", registered);
        });
        Arc::new(
            FilterCodec::new("IFDTestCodec", Arc::new(DummyCodec::new("Dummy")))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new()),
        )
    }

    fn info_stream() -> Arc<dyn InfoStream> {
        Arc::new(NoOutputInfoStream)
    }

    fn touch(dir: &dyn Directory, name: &str) {
        let mut out = dir
            .create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.close().unwrap();
    }

    /// Creates segment `name` with two data files, writes its `.si`, and returns
    /// the commit info.
    fn make_segment(dir: &dyn Directory, name: &str) -> SegmentCommitInfo {
        let mut info = test_segment_info(name, 10);
        info.set_codec(codec());
        // The shared helper defaults to `Sort::default()`, a SCORE sort the
        // segment-info format cannot serialise; give it a writable one.
        info.set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        let data = format!("{name}.fnm");
        let data2 = format!("{name}.fdt");
        touch(dir, &data);
        touch(dir, &data2);
        info.set_files(HashSet::from([data, data2]));
        Lucene99SegmentInfoFormat::new()
            .write(dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap()
    }

    /// Builds `SegmentInfos` over the named segments and commits it.
    fn commit_with(dir: &dyn Directory, segments: &[&str], counter: i64) -> SegmentInfos {
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = counter;
        for name in segments {
            sis.add(make_segment(dir, name)).unwrap();
        }
        sis.changed();
        sis.commit(dir).unwrap();
        sis
    }

    /// Builds a follow-on commit over `segments`, carrying `previous`'
    /// generation forward so the new `segments_N` does not collide.
    fn next_commit(
        dir: &dyn Directory,
        previous: &SegmentInfos,
        segments: &[&str],
        counter: i64,
    ) -> SegmentInfos {
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.update_generation(previous);
        sis.counter = counter;
        for name in segments {
            sis.add(make_segment(dir, name)).unwrap();
        }
        sis.changed();
        sis.commit(dir).unwrap();
        sis
    }

    fn deleter(
        dir: &Arc<dyn Directory>,
        policy: Arc<dyn IndexDeletionPolicy>,
        sis: &mut SegmentInfos,
    ) -> IndexFileDeleter {
        let files = dir.list_all().unwrap();
        IndexFileDeleter::new(
            &files,
            Arc::clone(dir),
            Arc::clone(dir),
            policy,
            sis,
            info_stream(),
            true,
            false,
        )
        .unwrap()
    }

    fn listing(dir: &dyn Directory) -> HashSet<String> {
        dir.list_all().unwrap().into_iter().collect()
    }

    // -- construction ---------------------------------------------------------

    #[test]
    fn init_adopts_the_commit_and_reference_counts_its_files() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);

        let d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        assert_eq!(d.commits().len(), 1);
        assert!(!d.starting_commit_deleted);
        // Each data file is referenced twice: once by the commit point loaded
        // from `segments_1`, and once by the closing `checkpoint(sis, false)`
        // the constructor always performs (IndexFileDeleter.java:241), which
        // protects the incoming SegmentInfos even when it is not the newest
        // commit. `segments_1` itself is referenced only by the commit point,
        // because a non-commit checkpoint excludes the segments file.
        for f in ["_0.si", "_0.fnm", "_0.fdt"] {
            assert_eq!(d.get_ref_count(f), 2, "{f}: commit point + checkpoint");
            assert!(d.exists(f));
        }
        assert_eq!(d.get_ref_count("segments_1"), 1);
        assert!(d.exists("segments_1"));
    }

    #[test]
    fn init_deletes_files_no_commit_references() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        // Debris from a crashed writer: matches the codec pattern, referenced by
        // nothing.
        touch(dir.as_ref(), "_9.fdt");
        touch(dir.as_ref(), "_9.si");
        assert!(listing(dir.as_ref()).contains("_9.fdt"));

        let _d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        let files = listing(dir.as_ref());
        assert!(!files.contains("_9.fdt"), "orphan must be deleted on init");
        assert!(!files.contains("_9.si"), "orphan must be deleted on init");
        assert!(files.contains("_0.fdt"), "referenced file must survive");
        assert!(files.contains("segments_1"));
    }

    #[test]
    fn init_leaves_unknown_non_codec_files_alone() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        touch(dir.as_ref(), "extra.txt");

        let _d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        assert!(
            listing(dir.as_ref()).contains("extra.txt"),
            "a file that is neither a codec file nor a segments file is not ours to delete"
        );
    }

    #[test]
    fn init_reconstructs_every_commit_generation_on_disk() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        sis.add(make_segment(dir.as_ref(), "_1")).unwrap();
        sis.counter = 2;
        sis.changed();
        sis.commit(dir.as_ref()).unwrap();

        // NoDeletionPolicy keeps both generations alive.
        let d = deleter(&dir, NoDeletionPolicy::instance(), &mut sis);

        let gens: Vec<i64> = d.commits().iter().map(|c| c.get_generation()).collect();
        assert_eq!(gens, vec![1, 2], "commits must be sorted oldest to newest");
        // `_0` is in both commits, plus the closing `checkpoint(sis, false)`.
        assert_eq!(d.get_ref_count("_0.fdt"), 3);
        // `_1` is only in the newest commit, plus that same checkpoint.
        assert_eq!(d.get_ref_count("_1.fdt"), 2);
    }

    #[test]
    fn init_drops_older_commits_under_keep_only_last() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let sis = commit_with(dir.as_ref(), &["_0"], 1);
        // Second commit uses a different segment, so gen 1's files become
        // unreferenced once gen 1 goes.
        let mut sis2 = next_commit(dir.as_ref(), &sis, &["_1"], 2);

        let d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis2);

        assert_eq!(d.commits().len(), 1);
        assert_eq!(d.commits()[0].get_generation(), 2);
        let files = listing(dir.as_ref());
        assert!(!files.contains("segments_1"), "old commit point removed");
        assert!(!files.contains("_0.fdt"), "its unreferenced files removed");
        assert!(files.contains("segments_2"));
        assert!(files.contains("_1.fdt"));
        let _ = sis;
    }

    // -- checkpoint -----------------------------------------------------------

    #[test]
    fn commit_checkpoint_retires_the_previous_commit_and_its_files() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        // Replace `_0` with `_1` and commit: gen 1 becomes collectable.
        let next = next_commit(dir.as_ref(), &sis, &["_1"], 2);

        // IndexWriter checkpoints the in-memory change first and only then
        // commits; only the non-commit branch releases `last_files`
        // (IndexFileDeleter.java:583-593), so both steps are needed for the
        // previous generation's files to become collectable.
        d.checkpoint(&next, false).unwrap();
        d.checkpoint(&next, true).unwrap();

        assert_eq!(d.commits().len(), 1, "policy kept only the newest commit");
        let files = listing(dir.as_ref());
        assert!(!files.contains("segments_1"));
        assert!(
            !files.contains("_0.fdt"),
            "unreferenced after the new commit"
        );
        assert!(files.contains("_1.fdt"));
    }

    #[test]
    fn non_commit_checkpoint_protects_then_releases_the_files() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        // An uncommitted (NRT) segment: on disk, referenced by no commit.
        let mut nrt = sis.clone();
        nrt.add(make_segment(dir.as_ref(), "_5")).unwrap();

        d.checkpoint(&nrt, false).unwrap();
        assert_eq!(d.get_ref_count("_5.fdt"), 1, "NRT file is protected");
        assert!(listing(dir.as_ref()).contains("_5.fdt"));

        // A later non-commit checkpoint that no longer names `_5` releases it.
        d.checkpoint(&sis, false).unwrap();
        assert_eq!(d.get_ref_count("_5.fdt"), 0);
        assert!(
            !listing(dir.as_ref()).contains("_5.fdt"),
            "released NRT file is deleted"
        );
    }

    #[test]
    fn close_releases_the_last_checkpoint() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        let mut nrt = sis.clone();
        nrt.add(make_segment(dir.as_ref(), "_5")).unwrap();
        d.checkpoint(&nrt, false).unwrap();
        assert_eq!(d.get_ref_count("_5.fdt"), 1);

        d.close().unwrap();
        assert_eq!(d.get_ref_count("_5.fdt"), 0);
        assert!(!listing(dir.as_ref()).contains("_5.fdt"));

        // Idempotent: closing twice must not double-decrement.
        d.close().unwrap();
        assert_eq!(d.get_ref_count("_5.fdt"), 0);
    }

    // -- pinning --------------------------------------------------------------

    #[test]
    fn a_snapshot_pins_its_commit_files_against_a_later_commit() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let policy = Arc::new(SnapshotDeletionPolicy::new(Arc::new(
            KeepOnlyLastCommitDeletionPolicy,
        )));
        let mut d = deleter(
            &dir,
            policy.clone() as Arc<dyn IndexDeletionPolicy>,
            &mut sis,
        );

        // Snapshot generation 1.
        let snapshot = policy.snapshot().unwrap();
        assert_eq!(snapshot.get_generation(), 1);

        // Commit a generation that shares nothing with gen 1.
        let next = next_commit(dir.as_ref(), &sis, &["_1"], 2);
        // IndexWriter checkpoints the in-memory change first and only then
        // commits; only the non-commit branch releases `last_files`
        // (IndexFileDeleter.java:583-593), so both steps are needed for the
        // previous generation's files to become collectable.
        d.checkpoint(&next, false).unwrap();
        d.checkpoint(&next, true).unwrap();

        let files = listing(dir.as_ref());
        assert!(
            files.contains("segments_1"),
            "the snapshotted commit point must survive"
        );
        assert!(
            files.contains("_0.fdt"),
            "files the snapshot pins must survive"
        );
        assert!(files.contains("segments_2"));

        // Releasing the snapshot and revisiting the policy collects it.
        policy.release(snapshot.as_ref()).unwrap();
        d.revisit_policy().unwrap();

        let files = listing(dir.as_ref());
        assert!(!files.contains("segments_1"), "released snapshot collected");
        assert!(!files.contains("_0.fdt"));
        assert!(files.contains("_1.fdt"));
    }

    #[test]
    fn files_pinned_by_a_reader_survive_the_commit_that_orphans_them() {
        // This is how IndexWriter pins an NRT reader's segment: it increments
        // the reader's file names directly (IndexWriter.java:4070,5274) and
        // decrements them when the reader is dropped (IndexWriter.java:5202).
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        let reader_files: Vec<String> = sis.info(0).files().unwrap().into_iter().collect();
        d.inc_ref_files(&reader_files);

        // Commit a generation that no longer contains `_0`.
        let next = next_commit(dir.as_ref(), &sis, &["_1"], 2);
        // IndexWriter checkpoints the in-memory change first and only then
        // commits; only the non-commit branch releases `last_files`
        // (IndexFileDeleter.java:583-593), so both steps are needed for the
        // previous generation's files to become collectable.
        d.checkpoint(&next, false).unwrap();
        d.checkpoint(&next, true).unwrap();

        let files = listing(dir.as_ref());
        assert!(
            files.contains("_0.fdt"),
            "a file pinned by a live reader must not be deleted"
        );
        assert!(
            !files.contains("segments_1"),
            "the commit point itself is not pinned by the reader"
        );

        // Closing the reader releases them.
        d.dec_ref_files(&reader_files).unwrap();
        assert!(!listing(dir.as_ref()).contains("_0.fdt"));
    }

    // -- refresh / new files --------------------------------------------------

    #[test]
    fn refresh_removes_unreferenced_files_left_by_a_rollback() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        // Debris a failed operation would leave behind.
        touch(dir.as_ref(), "_7.fdt");
        touch(dir.as_ref(), "pending_segments_3");

        d.refresh().unwrap();

        let files = listing(dir.as_ref());
        assert!(!files.contains("_7.fdt"));
        assert!(
            !files.contains("pending_segments_3"),
            "pending_segments is cleared during rollback"
        );
        assert!(files.contains("_0.fdt"), "referenced files untouched");
        assert!(files.contains("segments_1"));
    }

    #[test]
    fn delete_new_files_spares_referenced_files_and_is_idempotent() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = commit_with(dir.as_ref(), &["_0"], 1);
        let mut d = deleter(&dir, Arc::new(KeepOnlyLastCommitDeletionPolicy), &mut sis);

        touch(dir.as_ref(), "_8.fdt");
        let batch = vec!["_8.fdt".to_string(), "_0.fdt".to_string()];

        d.delete_new_files(&batch).unwrap();
        assert!(!listing(dir.as_ref()).contains("_8.fdt"));
        assert!(
            listing(dir.as_ref()).contains("_0.fdt"),
            "an incref'd file is never a 'new' file"
        );

        // Second call over the same batch: the file is already gone. Deletion
        // must remain safe and leave the bookkeeping consistent.
        let _ = d.delete_new_files(&batch);
        assert!(listing(dir.as_ref()).contains("_0.fdt"));
        // Commit point + the constructor's closing `checkpoint(sis, false)`.
        assert_eq!(d.get_ref_count("_0.fdt"), 2);
    }

    // -- inflate_gens ---------------------------------------------------------

    #[test]
    fn inflate_gens_pushes_the_counter_past_the_largest_segment_on_disk() {
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = 0;
        // `_z` is ordinal 35 in radix 36.
        let files = HashSet::from(["_z.fdt".to_string(), "_3.si".to_string()]);

        inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap();

        assert_eq!(sis.counter, 36, "counter must clear the largest name seen");
    }

    #[test]
    fn inflate_gens_pushes_the_generation_past_the_largest_segments_file() {
        let mut sis = SegmentInfos::new(10).unwrap();
        let files = HashSet::from([
            "segments_5".to_string(),
            "pending_segments_9".to_string(),
            "write.lock".to_string(),
        ]);

        inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap();

        // `pending_segments_9` is generation 9 (radix 36), and outranks
        // `segments_5`; the next write must clear it.
        assert_eq!(sis.generation(), 9);
    }

    #[test]
    fn inflate_gens_advances_per_segment_generations() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.add(make_segment(dir.as_ref(), "_0")).unwrap();
        assert_eq!(sis.info(0).get_next_del_gen(), 1);

        // A live-docs file at generation 4 exists on disk for `_0`.
        let files = HashSet::from(["_0.fdt".to_string(), "_0_4.liv".to_string()]);
        inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap();

        let info = sis.info(0);
        assert_eq!(info.get_next_del_gen(), 5);
        assert_eq!(info.get_next_field_infos_gen(), 5);
        assert_eq!(info.get_next_doc_values_gen(), 5);
    }

    #[test]
    fn inflate_gens_ignores_trash_and_temp_files() {
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = 0;
        let files = HashSet::from([
            // `!` is not a radix-36 digit, so neither name yields a generation.
            "segments_!!".to_string(),
            "pending_segments_!!".to_string(),
            "_5_9.tmp".to_string(),
            "_3.fdt".to_string(),
        ]);

        let generation_before = sis.generation();
        inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap();

        // `_5_9.tmp` is skipped, so `_3` is the largest name that counts.
        assert_eq!(sis.counter, 4);
        assert_eq!(
            sis.generation(),
            generation_before,
            "unparsable segments names yield no generation and leave it alone"
        );
    }

    #[test]
    fn inflate_gens_rejects_a_commit_naming_a_segment_with_no_files() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.add(make_segment(dir.as_ref(), "_0")).unwrap();

        // The listing does not mention `_0` at all.
        let files = HashSet::from(["segments_1".to_string()]);
        let err = inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap_err();

        assert!(
            matches!(err, LuceneError::CorruptIndex(_)),
            "expected a corrupt-index error, got {err:?}"
        );
    }

    #[test]
    fn inflate_gens_survives_a_non_ascii_pending_deletion_name() {
        // `IndexFileDeleter` folds the directory's pending-deletion set into the
        // names handed here, and that set is not filtered by the codec pattern.
        // A non-ASCII name must not panic (byte-slicing the leading character
        // would).
        // Java asserts that such a name starts with `_` (IndexFileDeleter.java:296)
        // and Rucene keeps that assertion, so the interesting case is a name
        // that satisfies it yet is still not ASCII.
        let mut sis = SegmentInfos::new(10).unwrap();
        let files = HashSet::from([
            "_é.si".to_string(),
            "_\u{10348}xyz.fdt".to_string(),
            "_0_é.liv".to_string(),
        ]);

        // The names carry no valid ordinal, so this reports a corrupt index.
        // The point of the test is that it does not panic: byte-slicing the
        // leading character used to abort the process here.
        let err = inflate_gens(&mut sis, &files, &NoOutputInfoStream).unwrap_err();
        assert!(matches!(err, LuceneError::CorruptIndex(_)), "got {err:?}");
    }
}
