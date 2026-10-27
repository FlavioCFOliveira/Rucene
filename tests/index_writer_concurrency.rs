//! `IndexWriter` under concurrent indexing, commits, merges and NRT reopens.
//!
//! Pins two writer defects against Apache Lucene Core 10.5.0:
//!
//! * **rmp #233.** `maybe_merge_with` held the writer's state lock while
//!   `TieredMergePolicy::find_merges` called back into
//!   `MergeContext::get_merging_segments`, which took the same lock again.
//!   Java's monitor is reentrant (`IndexWriter.updatePendingMerges` is
//!   `synchronized` and the policy calls `getMergingSegments()` from inside it);
//!   the port deadlocked the first time an NRT reopen asked for merges.
//! * **rmp #234.** Merges and commits failed with "file not found" under
//!   concurrent commits and flushes, for two reasons:
//!   * the merge asked `MergePolicy::use_compound_file` about a snapshot of
//!     the segments taken *after* releasing the writer's lock, so the policy
//!     sized segments a concurrent merge had already deleted; Java asks under
//!     the lock (`IndexWriter.java:5465-5467`, `:3594-3597`);
//!   * the commit held no reference on the files of the `SegmentInfos` it was
//!     committing, so a merge completing between `prepare_commit` and `commit`
//!     deleted them; Java takes `filesToCommit` (`IndexWriter.java:3842-3847`,
//!     `:4270`). A deterministic test pins this one.
//! * **Lost deletes and lost documents under concurrent merges** (defect B of
//!   the #234 audit), for three reasons, each a departure from
//!   `IndexWriter.java`:
//!   * `mergeInit` did not wait for the packets older than its segments
//!     (`bufferedUpdatesStream.waitApplyForMerge`, `:5035-5037`), so a packet
//!     applied after the merge reached the merged segment — whose
//!     buffered-deletes generation is the smallest of its sources' — and
//!     deleted documents it never saw;
//!   * a packet did not retry against a merge that committed while it was
//!     being resolved (`forceApply`, `:6320-6461`), and `commitMerge` did not
//!     carry over deletes, advance `mergeFinishedGen` and replace the segments
//!     under one hold of the lock (`:4669-4806`), so a delete resolved against
//!     a source segment in between was lost;
//!   * the merge readers were closed and the checkpoint taken after releasing
//!     the lock, so a packet resolving meanwhile referenced and released the
//!     merged segment's files before the deleter knew them.
//!
//! The scenario is the one the defects were found with: four indexing threads
//! updating and deleting documents while a control thread commits, forces
//! merges, asks for merges and reopens an NRT reader in a loop, with and
//! without an index sort. A watchdog turns a hang into a failure. The tests at
//! the end pin the commit protocol: the files a commit syncs and the
//! serialisation of concurrent commits (`commitLock`).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use rucene::codecs::Lucene104Codec;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeSpecification, MergeTrigger, TieredMergePolicy,
};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::{SegmentCommitInfo, SegmentInfos};
use rucene::search::{Sort, SortField, SortFieldType};
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

const INDEXERS: usize = 4;
const DOCS_PER_INDEXER: usize = 1000;
const IDS: usize = 200;
/// How long the whole scenario may take before it counts as a deadlock.
const WATCHDOG: Duration = Duration::from_secs(120);

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

fn id_term(id: usize) -> Term {
    Term::new("id", BytesRef::new(format!("id{id:04}").into_bytes()))
}

fn doc(id: usize, rank: i64) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{id:04}"), Store::NO).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("rank", rank)));
    d
}

/// Runs the scenario and checks that the final commit holds exactly the live
/// ids.
fn scenario(sorted: bool) {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_max_buffered_docs(10).unwrap();
    if sorted {
        config
            .set_index_sort(
                Sort::new_fields(vec![SortField::new(
                    Some("rank".to_string()),
                    SortFieldType::Long,
                )
                .unwrap()])
                .unwrap(),
            )
            .unwrap();
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let indexers: Vec<_> = (0..INDEXERS)
        .map(|t| {
            let writer = Arc::clone(&writer);
            thread::spawn(move || -> HashSet<usize> {
                let mut deleted = HashSet::new();
                for i in 0..DOCS_PER_INDEXER {
                    // Each thread owns the ids congruent to `t`, so the final
                    // live set is known without coordinating the threads.
                    let id = (i * INDEXERS + t) % IDS;
                    if i % 7 == 6 {
                        writer.delete_documents_by_terms(vec![id_term(id)]).unwrap();
                        deleted.insert(id);
                    } else {
                        writer
                            .update_document(id_term(id), &doc(id, (i * 31 % 97) as i64))
                            .unwrap();
                        deleted.remove(&id);
                    }
                }
                (0..IDS)
                    .filter(|id| id % INDEXERS == t && !deleted.contains(id))
                    .collect()
            })
        })
        .collect();

    let control = {
        let writer = Arc::clone(&writer);
        let done = Arc::clone(&done);
        thread::spawn(move || {
            let mut round = 0;
            while !done.load(Ordering::Acquire) {
                round += 1;
                writer.commit().unwrap();
                if round % 3 == 0 {
                    writer.force_merge(2).unwrap();
                }
                writer.force_merge_deletes().unwrap();
                writer.maybe_merge().unwrap();
                let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
                IndexReader::close(reader.as_ref()).unwrap();
            }
        })
    };

    let mut live = HashSet::new();
    for indexer in indexers {
        live.extend(indexer.join().expect("an indexing thread failed"));
    }
    done.store(true, Ordering::Release);
    control.join().expect("the control thread failed");

    writer.commit().unwrap();
    let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
    assert_eq!(
        reader.num_docs() as usize,
        live.len(),
        "every live id is in the index exactly once"
    );
    IndexReader::close(reader.as_ref()).unwrap();
    writer.close().unwrap();
}

/// Runs `body` on its own thread and fails when it does not finish within
/// [`WATCHDOG`].
fn run_with_watchdog(what: &str, body: impl FnOnce() + Send + std::panic::UnwindSafe + 'static) {
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let outcome = std::panic::catch_unwind(body);
        let _ = tx.send(outcome.is_ok());
    });
    match rx.recv_timeout(WATCHDOG) {
        Ok(true) => worker.join().unwrap(),
        Ok(false) => {
            let _ = worker.join();
            panic!("{what} failed");
        }
        Err(_) => panic!("the writer deadlocked in {what}: no progress in {WATCHDOG:?}"),
    }
}

/// An NRT reopen over a changed index asks the policy for merges
/// (`IndexWriter.getReader` -> `maybeMerge(FULL_FLUSH)`), and
/// `TieredMergePolicy.findMerges` asks the writer which segments are already
/// merging. That nested call re-entered the writer's lock and deadlocked
/// (rmp #233).
#[test]
fn an_nrt_reopen_over_changes_asks_for_merges_without_deadlocking() {
    run_with_watchdog("an NRT reopen", || {
        ensure_codec();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&doc(0, 0)).unwrap();
        let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
        assert_eq!(reader.num_docs(), 1);
        IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();
    });
}

#[test]
fn concurrent_updates_commits_merges_and_nrt_reopens_finish() {
    run_with_watchdog("the unsorted scenario", || scenario(false));
}

#[test]
fn concurrent_updates_commits_merges_and_nrt_reopens_finish_with_an_index_sort() {
    run_with_watchdog("the sorted scenario", || scenario(true));
}

/// A merge that completes between `prepare_commit` and `commit` must not
/// delete the files of the segments the pending commit names.
///
/// `IndexWriter.prepareCommitInternal` takes a reference on
/// `toCommit.files(false)` (`IndexWriter.java:3842-3847`) and `finishCommit`
/// releases it only once `segments_N` is written (`IndexWriter.java:4270`).
/// Without that reference the merge's checkpoint deleted the flushed segments
/// it merged away, and the commit then recorded segments whose files were
/// gone.
#[test]
fn a_merge_between_prepare_commit_and_commit_keeps_the_pending_commits_files() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for id in 0..30 {
        writer.add_document(&doc(id, id as i64)).unwrap();
        if id % 10 == 9 {
            writer.flush().unwrap();
        }
    }
    writer.prepare_commit().unwrap();
    // Merges the three flushed segments away while the commit is pending.
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir))
        .expect("every file the commit names is still in the directory");
    assert_eq!(reader.num_docs(), 30);
    IndexReader::close(reader.as_ref()).unwrap();
    writer.close().unwrap();
}

/// `TieredMergePolicy`, recording whether the writer's lock is held while the
/// writer asks it `use_compound_file`.
///
/// The probe asks the writer for `num_docs()`, which takes the writer's lock,
/// from another thread: when the lock is held the probe cannot finish before
/// `use_compound_file` returns.
struct LockProbePolicy {
    inner: TieredMergePolicy,
    writer: Arc<OnceLock<Weak<IndexWriter>>>,
    probed: AtomicBool,
    lock_held: AtomicBool,
}

impl fmt::Display for LockProbePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LockProbePolicy({})", self.inner)
    }
}

impl MergePolicy for LockProbePolicy {
    fn find_merges(
        &self,
        trigger: MergeTrigger,
        infos: &SegmentInfos,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        self.inner.find_merges(trigger, infos, context)
    }

    fn find_forced_merges(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_merges(infos, max_num_segments, segments_to_merge, context)
    }

    fn find_forced_deletes_merges(
        &self,
        infos: &SegmentInfos,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        self.inner.find_forced_deletes_merges(infos, context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        context: &dyn MergeContext,
    ) -> rucene::Result<bool> {
        if let Some(writer) = self.writer.get().and_then(Weak::upgrade) {
            let (tx, rx) = mpsc::channel();
            // Detached on purpose: while the lock is held the probe cannot
            // finish, and it finishes on its own once the lock is released.
            thread::spawn(move || {
                let _ = writer.num_docs();
                let _ = tx.send(());
            });
            let held = rx.recv_timeout(Duration::from_millis(500)).is_err();
            self.lock_held.store(held, Ordering::Release);
            self.probed.store(true, Ordering::Release);
        }
        self.inner.use_compound_file(infos, merged_info, context)
    }
}

/// The merge asks the policy whether to build a compound file while holding
/// the writer's lock.
///
/// `IndexWriter.mergeMiddle` calls `mergePolicy.useCompoundFile(segmentInfos,
/// merge.info, this)` inside `synchronized (this) { // Guard segmentInfos`
/// (`IndexWriter.java:5465-5467`): the policy sizes every live segment, and
/// only the lock keeps a concurrent merge from deleting one of them in the
/// meantime. Asked about a snapshot after releasing the lock, the policy read
/// the files of segments already merged away ("file not found", rmp #234).
#[test]
fn the_merge_asks_use_compound_file_under_the_writers_lock() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer_slot: Arc<OnceLock<Weak<IndexWriter>>> = Arc::new(OnceLock::new());
    let policy = Arc::new(LockProbePolicy {
        inner: TieredMergePolicy::new(),
        writer: Arc::clone(&writer_slot),
        probed: AtomicBool::new(false),
        lock_held: AtomicBool::new(false),
    });
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::clone(&policy) as Arc<dyn MergePolicy>)
        .unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let _ = writer_slot.set(Arc::downgrade(&writer));
    for id in 0..20 {
        writer.add_document(&doc(id, id as i64)).unwrap();
        if id % 10 == 9 {
            writer.flush().unwrap();
        }
    }
    writer.force_merge(1).unwrap();

    assert!(policy.probed.load(Ordering::Acquire), "the merge asked");
    assert!(
        policy.lock_held.load(Ordering::Acquire),
        "the writer's lock was held while the policy was asked"
    );
    writer.close().unwrap();
}

/// A `RamDirectory` that records every file name it is asked to sync.
struct SyncRecordingDirectory {
    inner: RamDirectory,
    synced: std::sync::Mutex<HashSet<String>>,
}

impl Directory for SyncRecordingDirectory {
    fn list_all(&self) -> rucene::Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> rucene::Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> rucene::Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(
        &self,
        name: &str,
        context: &dyn rucene::store::IOContext,
    ) -> rucene::Result<Box<dyn rucene::store::IndexOutput>> {
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn rucene::store::IOContext,
    ) -> rucene::Result<Box<dyn rucene::store::IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> rucene::Result<()> {
        self.synced.lock().unwrap().extend(names.iter().cloned());
        self.inner.sync(names)
    }
    fn sync_metadata(&self) -> rucene::Result<()> {
        self.inner.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> rucene::Result<()> {
        self.inner.rename(source, dest)
    }
    fn open_input(
        &self,
        name: &str,
        context: &dyn rucene::store::IOContext,
    ) -> rucene::Result<Box<dyn rucene::store::IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn obtain_lock(&self, name: &str) -> rucene::Result<Box<dyn rucene::store::Lock>> {
        self.inner.obtain_lock(name)
    }
    fn close(&mut self) -> rucene::Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> rucene::Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }
}

/// A commit syncs every file of the segments it records before it writes
/// `segments_N`.
///
/// `IndexWriter.startCommit` calls `directory.sync(toSync.files(false))`
/// (`IndexWriter.java:5774`): without it a crash after the commit could leave a
/// `segments_N` naming files that never reached stable storage.
#[test]
fn a_commit_syncs_every_file_of_its_segments() {
    ensure_codec();
    let recording = Arc::new(SyncRecordingDirectory {
        inner: RamDirectory::new(),
        synced: std::sync::Mutex::new(HashSet::new()),
    });
    let dir: Arc<dyn Directory> = Arc::clone(&recording) as Arc<dyn Directory>;
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for id in 0..10 {
        writer.add_document(&doc(id, id as i64)).unwrap();
    }
    writer.commit().unwrap();
    let infos = rucene::index::SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let files = infos.files(false).unwrap();
    assert!(!files.is_empty());
    let synced = recording.synced.lock().unwrap().clone();
    for file in &files {
        assert!(
            synced.contains(file),
            "{file} was not synced before the commit"
        );
    }
    writer.close().unwrap();
}

/// Two threads committing at once never interleave their commits.
///
/// `IndexWriter.commitInternal` runs under `commitLock`
/// (`IndexWriter.java:4224`); without it, one thread's `finishCommit` could
/// finish, or find missing, the commit the other prepared.
#[test]
fn concurrent_commits_are_serialised() {
    run_with_watchdog("concurrent commits", || {
        ensure_codec();
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let writer = Arc::clone(&writer);
                thread::spawn(move || {
                    for i in 0..100 {
                        writer.add_document(&doc(t * 1000 + i, i as i64)).unwrap();
                        writer.commit().unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("a committing thread failed");
        }
        writer.commit().unwrap();
        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.num_docs(), 400);
        IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();
    });
}
