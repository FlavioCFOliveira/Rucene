//! `ConcurrentMergeScheduler`, `MultiIndexMergeScheduler` and the merged-segment
//! warmer, against Apache Lucene Core 10.5.0.
//!
//! The cases port `TestConcurrentMergeScheduler` (lucene/core/src/test,
//! 10.5.0) where Rucene has the machinery they need, and pin the contracts of
//! `ConcurrentMergeScheduler.java`:
//!
//! * `merge` starts a `MergeThread` per merge and returns without waiting
//!   (`ConcurrentMergeScheduler.java:541-603`); `IndexWriter.close` and
//!   `waitForMerges` wait (`IndexWriter.java:1333-1334, 2845-2869`);
//! * `maybeStall` stalls the producer once `maxMergeCount` merges run and more
//!   are pending (`:614-645`; `testMaxMergeCount`);
//! * `updateMergeThreads` stops the biggest merges beyond `maxThreadCount` and
//!   `updateIOThrottle` retunes the auto throttle (`:312-420, 825-923`);
//! * a merge failure becomes a tragic event on the writer and reaches
//!   `handleMergeException` on the merge thread (`IndexWriter.java:4950-4965`,
//!   `ConcurrentMergeScheduler.java:755-774`);
//! * a tragic merge failure closes the writer through `rollbackInternal`,
//!   releasing the write lock and the writer itself
//!   (`IndexWriter.java:5876-5884`);
//! * `forceMerge` marks the merges already running, which then cascade
//!   (`IndexWriter.java:2229-2237, 4877-4883`); a forced merge that is aborted
//!   surfaces "background merge hit exception" (`:2253-2264`);
//! * a second `close` waits for the first (`IndexWriter.java:1382-1400`);
//! * `MultiIndexMergeScheduler.close` waits only for its own index's merges
//!   (`MultiIndexMergeScheduler.java:73-79, 160-193`);
//! * the `IndexReaderWarmer` runs once per merged segment, with reader pooling
//!   on (`IndexWriter.java:5562-5574`).
//!
//! Every case that waits on another thread runs under a watchdog, so a
//! deadlock fails the test instead of hanging it.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use rucene::codecs::Lucene104Codec;
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, MergedSegmentWarmer, OpenMode};
use rucene::index::leaf_reader::LeafReader;
use rucene::index::merge_policy::{
    LogMergePolicy, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, NoMergePolicy,
    OneMerge, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{
    CombinedMergeScheduler, ConcurrentMergeScheduler, MergeRateLimiter, MergeScheduler,
    MergeSource, MultiIndexMergeScheduler, SerialMergeScheduler, TaggedMergeSource,
    AUTO_DETECT_MERGES_AND_THREADS,
};
use rucene::index::sorting_codec_reader::SimpleMergedSegmentWarmer;
use rucene::index::terms::Term;
use rucene::index::SegmentInfos;
use rucene::store::{Directory, FSDirectory, RamDirectory};
use rucene::util::{BytesRef, InfoStream};

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

/// Runs `body` on its own thread and fails the test if it does not finish
/// within `seconds`: a deadlock becomes a failure, not a hang.
fn watchdog<T: Send + 'static>(seconds: u64, body: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let _ = tx.send(body());
    });
    match rx.recv_timeout(Duration::from_secs(seconds)) {
        Ok(value) => {
            let _ = handle.join();
            value
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => unreachable!("the body sent nothing and did not panic"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("watchdog: no progress after {seconds} s (deadlock or a wait that never ends)")
        }
    }
}

/// A gate merges wait on until the test opens it.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

fn id_doc(id: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", id.to_string(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        TextField::new("content", "a b c".to_string(), Store::NO).unwrap(),
    ));
    doc
}

fn log_doc_policy(merge_factor: i32) -> Arc<LogMergePolicy> {
    let mut policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(merge_factor).unwrap();
    Arc::new(policy)
}

fn committed_segment_count(dir: &Arc<dyn Directory>) -> usize {
    SegmentInfos::read_latest_commit(Arc::clone(dir))
        .unwrap()
        .size()
}

/// A `ConcurrentMergeScheduler` whose `doMerge` waits on `gate` before merging
/// and counts the merges that started and finished.
fn gated_cms(
    gate: &Arc<Gate>,
    started: &Arc<AtomicUsize>,
    finished: &Arc<AtomicUsize>,
) -> Arc<ConcurrentMergeScheduler> {
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    let (gate, started, finished) = (Arc::clone(gate), Arc::clone(started), Arc::clone(finished));
    cms.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
        started.fetch_add(1, Ordering::SeqCst);
        gate.wait();
        let result = cms.default_do_merge(source, merge);
        finished.fetch_add(1, Ordering::SeqCst);
        result
    })));
    cms
}

/// Two one-document commits under `LogDocMergePolicy(mergeFactor=2)`: the
/// second commit's flush queues a merge of the two segments.
fn writer_with_a_queued_merge(
    scheduler: Arc<dyn MergeScheduler>,
) -> (Arc<dyn Directory>, Arc<IndexWriter>) {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_merge_policy(log_doc_policy(2)).unwrap();
    config.set_merge_scheduler(scheduler).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.commit().unwrap();
    (dir, writer)
}

#[test]
fn merge_returns_while_the_merge_runs_and_close_waits_for_it() {
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let cms = gated_cms(&gate, &started, &finished);
    let (dir, writer) = {
        let cms = Arc::clone(&cms);
        // The commits return although the merge they queued is blocked: Java's
        // `merge` hands the merge to a thread and returns.
        watchdog(60, move || writer_with_a_queued_merge(cms))
    };
    watchdog(60, {
        let started = Arc::clone(&started);
        move || {
            while started.load(Ordering::SeqCst) == 0 {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
    assert_eq!(finished.load(Ordering::SeqCst), 0);
    assert_eq!(cms.merge_thread_count(), 1, "one merge thread is running");
    assert_eq!(writer.get_segment_count().unwrap(), 2);

    // `close` waits for the running merge (`shutdown` → `waitForMerges`).
    let (closed_tx, closed_rx) = mpsc::channel();
    let closer = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || {
            let outcome = writer.close();
            let _ = closed_tx.send(());
            outcome
        })
    };
    assert!(
        closed_rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "close returned while a merge was still running"
    );
    gate.open();
    closed_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("close finished once the merge could");
    closer.join().unwrap().unwrap();
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    assert!(cms.merge_threads().iter().all(|t| !t.is_alive()));
    assert_eq!(
        committed_segment_count(&dir),
        1,
        "close committed the merge"
    );
}

#[test]
fn force_merge_without_waiting_returns_and_wait_for_merges_waits() {
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let cms = gated_cms(&gate, &started, &finished);
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_merge_policy(log_doc_policy(10)).unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..3 {
        writer.add_document(&id_doc(i)).unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        0,
        "three segments do not reach mergeFactor=10"
    );

    // `forceMerge(1, false)` returns while the merge it started is blocked
    // (`IndexWriter.java:2283-2285`).
    let writer2 = Arc::clone(&writer);
    watchdog(60, move || writer2.force_merge_with(1, false)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let waiter = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || {
            let outcome = writer.wait_for_merges();
            let _ = done_tx.send(());
            outcome
        })
    };
    assert!(
        done_rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "waitForMerges returned while a merge was still running"
    );
    gate.open();
    done_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("waitForMerges finished once the merge could");
    waiter.join().unwrap().unwrap();
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    assert_eq!(writer.get_segment_count().unwrap(), 1);
    watchdog(60, move || writer.close()).unwrap();
}

/// `TestConcurrentMergeScheduler.testMaxMergeCount` (LUCENE-4544): while
/// `maxMergeCount` merges run and more are pending, the indexing thread is
/// stalled, so no more than `maxMergeCount` merges ever run at once.
#[test]
fn max_merge_count_stalls_the_producer() {
    ensure_codec();
    for (max_merge_count, max_merge_threads) in [(1, 1), (2, 1), (3, 2), (5, 5)] {
        watchdog(120, move || {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let enough_merges_waiting = Arc::new(AtomicI32::new(max_merge_count));
            let running_merge_count = Arc::new(AtomicI32::new(0));
            let failed = Arc::new(AtomicBool::new(false));

            let cms = Arc::new(ConcurrentMergeScheduler::new());
            {
                let enough = Arc::clone(&enough_merges_waiting);
                let running = Arc::clone(&running_merge_count);
                let failed_in_merge = Arc::clone(&failed);
                cms.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
                    let failed = &failed_in_merge;
                    // Stall all incoming merges until we see maxMergeCount.
                    let count = running.fetch_add(1, Ordering::SeqCst) + 1;
                    let outcome = (|| {
                        if count > max_merge_count {
                            failed.store(true, Ordering::SeqCst);
                            return Err(LuceneError::illegal_state(format!(
                                "count={count} vs maxMergeCount={max_merge_count}"
                            )));
                        }
                        enough.fetch_sub(1, Ordering::SeqCst);
                        // Stall this merge until we see exactly maxMergeCount
                        // merges waiting.
                        while enough.load(Ordering::SeqCst) > 0 && !failed.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(10));
                        }
                        // Then sleep a bit to give a chance for the bug (too
                        // many pending merges) to appear.
                        thread::sleep(Duration::from_millis(20));
                        cms.default_do_merge(source, merge.clone())
                    })();
                    running.fetch_sub(1, Ordering::SeqCst);
                    if outcome.is_err() {
                        source.on_merge_finished(&merge);
                    }
                    outcome
                })));
                let failed = Arc::clone(&failed);
                cms.set_handle_merge_exception_override(Some(Arc::new(move |_error| {
                    failed.store(true, Ordering::SeqCst);
                })));
            }
            cms.set_max_merges_and_threads(max_merge_count, max_merge_threads)
                .unwrap();

            let mut config = IndexWriterConfig::new();
            config.set_commit_on_close(false);
            config
                .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
                .unwrap();
            config.set_max_buffered_docs(2).unwrap();
            let mut tmp = TieredMergePolicy::new();
            tmp.set_max_merge_at_once(2).unwrap();
            tmp.set_segments_per_tier(2.0).unwrap();
            config.set_merge_policy(Arc::new(tmp)).unwrap();

            let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("field", "field".to_string(), Store::NO).unwrap(),
            ));
            while enough_merges_waiting.load(Ordering::SeqCst) > 0 && !failed.load(Ordering::SeqCst)
            {
                for _ in 0..10 {
                    writer.add_document(&doc).unwrap();
                }
            }
            let commit = writer.commit();
            let close = writer.close();
            assert!(
                !failed.load(Ordering::SeqCst),
                "more than maxMergeCount={max_merge_count} merges ran at once"
            );
            commit.unwrap();
            close.unwrap();
        });
    }
}

/// `testInvalidMaxMergeCountAndThreads`.
#[test]
fn invalid_max_merge_count_and_threads_are_rejected() {
    let cms = ConcurrentMergeScheduler::new();
    assert!(matches!(
        cms.set_max_merges_and_threads(AUTO_DETECT_MERGES_AND_THREADS, 3),
        Err(LuceneError::IllegalArgument(_))
    ));
    assert!(matches!(
        cms.set_max_merges_and_threads(3, AUTO_DETECT_MERGES_AND_THREADS),
        Err(LuceneError::IllegalArgument(_))
    ));
}

/// `testDynamicDefaults` and `testResetToAutoDefault`.
#[test]
fn auto_detected_counts_resolve_at_initialize_and_reset_to_the_sentinel() {
    ensure_codec();
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    assert_eq!(cms.get_max_merge_count(), AUTO_DETECT_MERGES_AND_THREADS);
    assert_eq!(cms.get_max_thread_count(), AUTO_DETECT_MERGES_AND_THREADS);
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let cores = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let threads = (cores / 2).max(1) as i32;
    assert_eq!(cms.get_max_thread_count(), threads);
    assert_eq!(cms.get_max_merge_count(), threads + 5);
    writer.close().unwrap();

    cms.set_max_merges_and_threads(4, 3).unwrap();
    assert_eq!(cms.get_max_merge_count(), 4);
    assert_eq!(cms.get_max_thread_count(), 3);
    cms.set_max_merges_and_threads(
        AUTO_DETECT_MERGES_AND_THREADS,
        AUTO_DETECT_MERGES_AND_THREADS,
    )
    .unwrap();
    assert_eq!(cms.get_max_merge_count(), AUTO_DETECT_MERGES_AND_THREADS);
    assert_eq!(cms.get_max_thread_count(), AUTO_DETECT_MERGES_AND_THREADS);
}

/// `testDeleteMerging`: deletes interleaved with background merges are not
/// lost.
#[test]
fn deletes_survive_background_merges() {
    ensure_codec();
    watchdog(120, || {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut policy = LogMergePolicy::by_doc_count();
        // Force degenerate merging so we can get a mix of merging of segments
        // with and without deletes at the start.
        policy.set_min_merge_docs(1000);
        let mut config = IndexWriterConfig::new();
        config.set_merge_policy(Arc::new(policy)).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..10 {
            for j in 0..100 {
                writer.add_document(&id_doc(i * 100 + j)).unwrap();
            }
            let mut del_id = i;
            while del_id < 100 * (1 + i) {
                writer
                    .delete_documents_by_terms(vec![Term::new(
                        "id",
                        BytesRef::new(del_id.to_string().into_bytes()),
                    )])
                    .unwrap();
                del_id += 10;
            }
            writer.commit().unwrap();
        }
        writer.close().unwrap();
        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        // Verify that we did not lose any deletes.
        assert_eq!(reader.num_docs(), 450);
        IndexReader::close(reader.as_ref()).unwrap();
    });
}

/// `testNoWaitClose`: with `commitOnClose=false`, `close` rolls back and
/// aborts the running merges (`abortMerges`), and the last commit survives.
///
/// To make sure a merge is running when `close` is called, the first merge
/// of each writer is held until it is aborted; the test counts the merges
/// that ended aborted and requires one per writer.
#[test]
fn rollback_aborts_running_merges_and_keeps_the_last_commit() {
    ensure_codec();
    watchdog(180, || {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let held_started = Arc::new(AtomicBool::new(false));
        let aborted = Arc::new(AtomicUsize::new(0));
        let config = |open_mode: OpenMode| {
            // Holds the first merge of this writer until the rollback aborts it.
            let hold_next = Arc::new(AtomicBool::new(true));
            let cms = Arc::new(ConcurrentMergeScheduler::new());
            let (held_started, aborted) = (Arc::clone(&held_started), Arc::clone(&aborted));
            cms.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
                let probe = merge.clone();
                if hold_next.swap(false, Ordering::SeqCst) {
                    held_started.store(true, Ordering::SeqCst);
                    while !probe.is_aborted() {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                let result = cms.default_do_merge(source, merge);
                if probe.is_aborted() {
                    aborted.fetch_add(1, Ordering::SeqCst);
                }
                result
            })));
            let mut config = IndexWriterConfig::new();
            // Force excessive merging.
            config.set_max_buffered_docs(2).unwrap();
            config.set_merge_policy(log_doc_policy(3)).unwrap();
            config.set_merge_scheduler(cms).unwrap();
            config.set_commit_on_close(false);
            config.set_open_mode(open_mode).unwrap();
            config
        };
        let mut writer = IndexWriter::new(Arc::clone(&dir), config(OpenMode::CREATE)).unwrap();
        for iter in 0..3 {
            held_started.store(false, Ordering::SeqCst);
            let aborted_before = aborted.load(Ordering::SeqCst);
            for j in 0..201 {
                writer.add_document(&id_doc(iter * 201 + j)).unwrap();
            }
            let mut del_id = iter * 201;
            for _ in 0..20 {
                writer
                    .delete_documents_by_terms(vec![Term::new(
                        "id",
                        BytesRef::new(del_id.to_string().into_bytes()),
                    )])
                    .unwrap();
                del_id += 5;
            }
            // Force a bunch of merge threads to kick off so we stress out
            // aborting them on close.
            writer.add_document(&id_doc(-1)).unwrap();
            let commit = writer.commit();
            while !held_started.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            writer.close().unwrap();
            commit.unwrap();
            assert!(
                aborted.load(Ordering::SeqCst) > aborted_before,
                "close aborted no running merge"
            );

            let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
            assert_eq!(reader.num_docs(), (1 + iter) * 182);
            IndexReader::close(reader.as_ref()).unwrap();

            writer = IndexWriter::new(Arc::clone(&dir), config(OpenMode::APPEND)).unwrap();
        }
        writer.close().unwrap();
    });
}

/// A merge failure inside `IndexWriter.merge` is recorded as a tragic event on
/// the writer, and the merge thread hands the error to `handleMergeException`.
#[test]
fn a_merge_exception_reaches_the_writer_and_the_merge_thread() {
    #[derive(Debug)]
    struct FailingWarmer;
    impl std::fmt::Display for FailingWarmer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailingWarmer")
        }
    }
    impl MergedSegmentWarmer for FailingWarmer {
        fn warm(&self, _reader: &dyn LeafReader) -> Result<()> {
            Err(LuceneError::Io(std::io::Error::other(
                "warmer failed on purpose",
            )))
        }
    }

    ensure_codec();
    let handled: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    {
        let handled = Arc::clone(&handled);
        cms.set_handle_merge_exception_override(Some(Arc::new(move |error| {
            handled.lock().unwrap().push(error.to_string());
        })));
    }
    // A file-system directory, whose write lock is process-wide: a writer
    // that keeps it makes the reopen below fail.
    let tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let mut config = IndexWriterConfig::new();
    config.set_merge_policy(log_doc_policy(2)).unwrap();
    config.set_reader_pooling(true);
    config
        .set_merged_segment_warmer(Some(Arc::new(FailingWarmer)))
        .unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    // This commit queues the merge; the merge fails on its own thread, and may
    // do so before the commit finishes, which then fails on the tragedy or on
    // the writer the tragedy closed.
    if let Err(error) = writer.commit() {
        assert!(
            matches!(error, LuceneError::AlreadyClosed { .. })
                || error.to_string().contains("unrecoverable"),
            "{error}"
        );
    }
    {
        let cms = Arc::clone(&cms);
        watchdog(60, move || cms.sync());
    }

    let tragedy = writer
        .get_tragic_exception()
        .expect("the failed merge is a tragic event on the writer");
    assert!(
        tragedy.to_string().contains("warmer failed on purpose"),
        "{tragedy}"
    );
    assert!(writer.ensure_open().is_err(), "a tragic writer is closed");
    // `maybeCloseOnTragicEvent` rolled the writer back: it is closed, not
    // merely refusing work (`IndexWriter.java:5876-5884`).
    assert!(writer.is_closed());
    // `hasPendingMerges` and `getNextMerge` refuse to hand out merges once
    // the writer is tragic (`IndexWriter.java:2498-2524`).
    match writer.has_pending_merges_pub() {
        // `new IllegalStateException("this writer hit an unrecoverable error;
        // cannot merge", tragedy.get())`: Java's message, and the tragedy as
        // the cause.
        Err(LuceneError::IllegalState { message, cause }) => {
            assert_eq!(
                message,
                "this writer hit an unrecoverable error; cannot merge"
            );
            assert!(
                cause
                    .as_ref()
                    .is_some_and(|cause| Arc::ptr_eq(cause, &tragedy)),
                "the cause is the tragedy: {cause:?}"
            );
        }
        other => panic!("expected IllegalStateException, got {other:?}"),
    }
    {
        let handled = handled.lock().unwrap();
        assert_eq!(
            handled.len(),
            1,
            "handleMergeException ran once: {handled:?}"
        );
        assert!(
            handled[0].contains("warmer failed on purpose"),
            "{handled:?}"
        );
    }
    // The terminated merge thread no longer holds the writer: the test's
    // handle is the last one, and dropping it drops the writer.
    assert_eq!(
        Arc::strong_count(&writer),
        1,
        "the writer is still referenced"
    );
    let weak = Arc::downgrade(&writer);
    drop(writer);
    assert!(weak.upgrade().is_none(), "the writer was not dropped");

    // The rollback released the write lock, so the index opens again; the new
    // writer's deleter removes what the failed merge left behind, as
    // `IndexFileDeleter`'s constructor does in Java.
    let reopened_dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).unwrap());
    let mut config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::APPEND).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let reopened = IndexWriter::new(Arc::clone(&reopened_dir), config)
        .expect("the write lock was released by the tragic rollback");
    let commit = SegmentInfos::read_latest_commit(Arc::clone(&reopened_dir)).unwrap();
    let mut referenced = commit.files(true).unwrap();
    referenced.insert("write.lock".to_string());
    let leftovers: Vec<String> = reopened_dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| !referenced.contains(name))
        .collect();
    assert!(
        leftovers.is_empty(),
        "unreferenced files remain: {leftovers:?}"
    );
    reopened.close().unwrap();
}

/// Without an override, `handleMergeException` throws `MergeException` out of
/// the merge thread; the Rust counterpart is a panic carrying it.
#[test]
fn the_default_handle_merge_exception_throws_a_merge_exception() {
    let cms = ConcurrentMergeScheduler::new();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cms.handle_merge_exception(LuceneError::Io(std::io::Error::other("disk full")))
    }))
    .expect_err("the merge exception escapes");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("disk full"), "{message}");
}

/// A merge source the test fills with merges of a chosen estimated size; each
/// merge blocks on a gate.
struct FakeMergeSource {
    pending: Mutex<Vec<OneMerge>>,
    gate: Arc<Gate>,
    finished: AtomicUsize,
}

impl std::fmt::Display for FakeMergeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeMergeSource")
    }
}

impl MergeSource for FakeMergeSource {
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            Ok(None)
        } else {
            Ok(Some(pending.remove(0)))
        }
    }

    fn on_merge_finished(&self, _merge: &OneMerge) {
        self.finished.fetch_add(1, Ordering::SeqCst);
    }

    fn has_pending_merges(&self) -> Result<bool> {
        Ok(!self.pending.lock().unwrap().is_empty())
    }

    fn merge(&self, merge: OneMerge) -> Result<()> {
        self.gate.wait();
        self.on_merge_finished(&merge);
        Ok(())
    }
}

fn merge_of_mb(mb: i64) -> OneMerge {
    let mut merge = OneMerge::from_codec_readers(Vec::new());
    merge.estimated_merge_bytes = mb * 1024 * 1024;
    merge
}

/// Captures every `"MS"` message.
#[derive(Default)]
struct CapturingInfoStream {
    messages: Mutex<Vec<String>>,
}

impl InfoStream for CapturingInfoStream {
    fn java_class_name(&self) -> &'static str {
        "CapturingInfoStream"
    }

    fn message(&self, _component: &str, message: &str) {
        self.messages.lock().unwrap().push(message.to_string());
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "MS"
    }

    fn close(&self) {}
}

/// `updateMergeThreads`: with `maxThreadCount=1`, the bigger of two big merges
/// is stopped and the smaller runs; `updateIOThrottle` lowers the auto target
/// by 10% when there is no backlog; `testMergeThreadMessages`' launch message.
#[test]
fn big_merges_beyond_max_thread_count_are_stopped_and_the_throttle_retunes() {
    let gate = Arc::new(Gate::default());
    let source = Arc::new(FakeMergeSource {
        pending: Mutex::new(vec![merge_of_mb(200), merge_of_mb(100)]),
        gate: Arc::clone(&gate),
        finished: AtomicUsize::new(0),
    });
    let cms = ConcurrentMergeScheduler::new();
    let info_stream = Arc::new(CapturingInfoStream::default());
    cms.set_info_stream(Arc::clone(&info_stream) as Arc<dyn InfoStream>);
    cms.set_max_merges_and_threads(3, 1).unwrap();
    cms.enable_auto_io_throttle().unwrap();

    let source_dyn: Arc<dyn MergeSource> = Arc::clone(&source) as Arc<dyn MergeSource>;
    MergeScheduler::merge(&cms, source_dyn, MergeTrigger::Explicit).unwrap();

    let threads = cms.merge_threads();
    assert_eq!(threads.len(), 2);
    let rate_of = |mb: i64| {
        threads
            .iter()
            .find(|t| t.merge().estimated_merge_bytes == mb * 1024 * 1024)
            .unwrap()
            .rate_limiter()
            .get_mb_per_sec()
    };
    assert_eq!(
        rate_of(200),
        0.0,
        "the biggest merge beyond maxThreadCount is stopped"
    );
    // The first merge of 50 MB or more started without backlog, lowering the
    // target by 10%; the second found more merges than threads, a backlog
    // that leaves the target alone.
    let target = 20.0 / 1.1;
    assert!((cms.get_io_rate_limit_mb_per_sec() - target).abs() < 1e-9);
    assert!(
        (rate_of(100) - target).abs() < 1e-9,
        "the running merge is throttled to the target"
    );

    let messages = info_stream.messages.lock().unwrap().clone();
    assert!(messages
        .iter()
        .any(|m| m == "    launch new thread [Lucene Merge Thread #0]"));
    assert!(messages
        .iter()
        .any(|m| m == "    launch new thread [Lucene Merge Thread #1]"));
    assert!(
        messages.iter().any(|m| m.contains("\n  now stop\n")),
        "{messages:?}"
    );

    gate.open();
    watchdog(60, move || cms.sync());
    assert_eq!(source.finished.load(Ordering::SeqCst), 2);
}

/// `MultiIndexMergeScheduler`: merges of every index run on the combined
/// scheduler, tagged with their directory, and closing one index waits only
/// for that index's merges.
#[test]
fn multi_index_close_waits_only_for_its_own_index() {
    ensure_codec();
    let combined = Arc::new(CombinedMergeScheduler::new());
    // `TestMultiIndexMergeScheduler` sets the counts itself: no `IndexWriter`
    // initializes a combined scheduler, so they would stay at the
    // auto-detect sentinel.
    combined.set_default_max_merges_and_threads(false);
    let gate_b = Arc::new(Gate::default());
    let dir_a: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let dir_b: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let b_started = Arc::new(AtomicUsize::new(0));
    {
        let gate_b = Arc::clone(&gate_b);
        let dir_b = Arc::clone(&dir_b);
        let b_started = Arc::clone(&b_started);
        combined.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
            let any: &dyn std::any::Any = source.as_ref();
            let tagged = any
                .downcast_ref::<TaggedMergeSource>()
                .expect("a MultiIndexMergeScheduler tags its sources");
            if std::ptr::addr_eq(Arc::as_ptr(tagged.get_directory()), Arc::as_ptr(&dir_b)) {
                b_started.fetch_add(1, Ordering::SeqCst);
                gate_b.wait();
            }
            cms.default_do_merge(source, merge)
        })));
    }
    let open = |dir: &Arc<dyn Directory>| {
        let mut config = IndexWriterConfig::new();
        config.set_merge_policy(log_doc_policy(2)).unwrap();
        config
            .set_merge_scheduler(Arc::new(
                MultiIndexMergeScheduler::with_combined_merge_scheduler(
                    Arc::clone(dir),
                    Arc::clone(&combined),
                ),
            ))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(dir), config).unwrap();
        writer.add_document(&id_doc(0)).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_doc(1)).unwrap();
        writer.commit().unwrap();
        writer
    };
    let writer_b = open(&dir_b);
    {
        let b_started = Arc::clone(&b_started);
        watchdog(60, move || {
            while b_started.load(Ordering::SeqCst) == 0 {
                thread::sleep(Duration::from_millis(5));
            }
        });
    }
    let writer_a = open(&dir_a);
    // Index B's merge is still blocked; closing A must not wait for it.
    watchdog(60, move || writer_a.close()).unwrap();
    assert_eq!(committed_segment_count(&dir_a), 1);
    assert!(combined.merge_threads().iter().any(|t| t.is_alive()));

    gate_b.open();
    watchdog(60, move || writer_b.close()).unwrap();
    assert_eq!(committed_segment_count(&dir_b), 1);
}

/// Counts the segments it warms, delegating to `SimpleMergedSegmentWarmer`.
#[derive(Debug)]
struct CountingWarmer {
    inner: SimpleMergedSegmentWarmer,
    warmed: Mutex<Vec<i32>>,
}

impl std::fmt::Display for CountingWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CountingWarmer")
    }
}

impl MergedSegmentWarmer for CountingWarmer {
    fn warm(&self, reader: &dyn LeafReader) -> Result<()> {
        self.inner.warm(reader)?;
        self.warmed.lock().unwrap().push(reader.max_doc());
        Ok(())
    }
}

/// `IndexWriter.mergeMiddle` warms each merged segment once, through a pooled
/// reader, when reader pooling is on — and not at all when it is off.
#[test]
fn the_warmer_runs_once_per_merged_segment_when_readers_are_pooled() {
    ensure_codec();
    for (scheduler, pooling) in [
        (
            Arc::new(SerialMergeScheduler::new()) as Arc<dyn MergeScheduler>,
            true,
        ),
        (
            Arc::new(ConcurrentMergeScheduler::new()) as Arc<dyn MergeScheduler>,
            true,
        ),
        (
            Arc::new(ConcurrentMergeScheduler::new()) as Arc<dyn MergeScheduler>,
            false,
        ),
    ] {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        // Four one-document segments, written without merging.
        {
            let mut config = IndexWriterConfig::new();
            config
                .set_merge_policy(Arc::new(NoMergePolicy::new()))
                .unwrap();
            let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
            for i in 0..4 {
                writer.add_document(&id_doc(i)).unwrap();
                writer.commit().unwrap();
            }
            writer.close().unwrap();
        }
        let warmer = Arc::new(CountingWarmer {
            inner: SimpleMergedSegmentWarmer::new(Arc::new(rucene::util::NoOutputInfoStream)),
            warmed: Mutex::new(Vec::new()),
        });
        let mut config = IndexWriterConfig::new();
        config.set_open_mode(OpenMode::APPEND).unwrap();
        config.set_merge_policy(log_doc_policy(10)).unwrap();
        config.set_reader_pooling(pooling);
        config
            .set_merged_segment_warmer(Some(Arc::clone(&warmer) as Arc<dyn MergedSegmentWarmer>))
            .unwrap();
        config.set_merge_scheduler(scheduler).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        // One forced merge of the four segments: one merged segment of four
        // documents.
        watchdog(60, move || {
            writer.force_merge(1)?;
            writer.close()
        })
        .unwrap();
        assert_eq!(committed_segment_count(&dir), 1);
        let warmed = warmer.warmed.lock().unwrap().clone();
        if pooling {
            assert_eq!(warmed, vec![4], "one warm per merged segment");
        } else {
            assert!(
                warmed.is_empty(),
                "Java warms only pooled readers: {warmed:?}"
            );
        }
    }
}

/// `forceMerge` with the default scheduler and no merge policy cascade: the
/// writer, not the scheduler, waits (`IndexWriter.java:2243-2273`).
#[test]
fn force_merge_waits_for_background_merges() {
    ensure_codec();
    watchdog(120, || {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut config = IndexWriterConfig::new();
        config.set_merge_policy(log_doc_policy(2)).unwrap();
        config.set_max_buffered_docs(2).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..50 {
            writer.add_document(&id_doc(i)).unwrap();
        }
        writer.force_merge(1).unwrap();
        assert_eq!(writer.get_segment_count().unwrap(), 1);
        writer.close().unwrap();
    });
}

/// Polls `condition` every 5 ms for at most `seconds`; returns whether it
/// held.
fn eventually(seconds: u64, condition: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while std::time::Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    condition()
}

/// `forceMerge` marks the merges already running (`IndexWriter.java:2229-2237`)
/// on the one `OneMerge` object the merge thread runs, so a merge that started
/// before the call asks the policy for the next forced merge when it finishes
/// (`IndexWriter.java:4877-4883`), and `forceMerge(1)` ends with one segment.
#[test]
fn force_merge_marks_a_running_merge_which_then_cascades() {
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let cms = gated_cms(&gate, &started, &finished);
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    // Merge factor 2 and no minimum merge size: `_0` and `_1` merge, and the
    // merged two-document segment and `_2` sit on different levels, so no
    // natural merge follows.
    let mut policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(2).unwrap();
    policy.set_min_merge_docs(1);
    config.set_merge_policy(Arc::new(policy)).unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..2 {
        writer.add_document(&id_doc(i)).unwrap();
        writer.commit().unwrap();
    }
    assert!(
        eventually(60, || started.load(Ordering::SeqCst) == 1),
        "the merge of _0 and _1 started"
    );
    writer.add_document(&id_doc(2)).unwrap();
    writer.commit().unwrap();
    assert_eq!(writer.get_segment_count().unwrap(), 3);

    let (done_tx, done_rx) = mpsc::channel();
    let force_merger = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || {
            let outcome = writer.force_merge(1);
            let _ = done_tx.send(());
            outcome
        })
    };
    // The running merge carries the forced target as soon as `forceMerge`
    // marks it. Under a copy that does not share it, this times out and the
    // assertions below fail.
    let marked = eventually(5, || {
        cms.merge_threads()
            .iter()
            .any(|t| t.is_alive() && t.merge().max_num_segments() == 1)
    });
    gate.open();
    done_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("forceMerge returned");
    force_merger.join().unwrap().unwrap();
    assert!(marked, "forceMerge did not mark the running merge");
    assert_eq!(
        writer.get_segment_count().unwrap(),
        1,
        "forceMerge(1) returned before the running merge cascaded"
    );
    assert_eq!(started.load(Ordering::SeqCst), 2, "one cascading merge ran");
    watchdog(60, move || writer.close()).unwrap();
}

/// `shouldClose(true)` (`IndexWriter.java:1382-1400`): a second `close` waits
/// until the first one has closed the writer, and returns without closing it
/// again.
#[test]
fn a_concurrent_close_waits_for_the_first_close() {
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let cms = gated_cms(&gate, &started, &finished);
    let (dir, writer) = {
        let cms = Arc::clone(&cms);
        watchdog(60, move || writer_with_a_queued_merge(cms))
    };
    assert!(eventually(60, || started.load(Ordering::SeqCst) == 1));

    // The first close commits, so it waits for the blocked merge.
    let first = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || writer.close())
    };
    assert!(
        eventually(60, || !writer.is_open() && !writer.is_closed()),
        "the first close is under way"
    );
    let (second_tx, second_rx) = mpsc::channel();
    let second = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || {
            let outcome = writer.close();
            let _ = second_tx.send(writer.is_closed());
            outcome
        })
    };
    assert!(
        second_rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "the second close returned while the first was still closing"
    );
    gate.open();
    let closed_when_second_returned = second_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("the second close returned once the first finished");
    assert!(closed_when_second_returned);
    second.join().unwrap().unwrap();
    first.join().unwrap().unwrap();
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    assert_eq!(
        committed_segment_count(&dir),
        1,
        "the first close committed"
    );
}

/// `forceMerge` forwards the failure of a forced merge to its caller, an
/// abort included (`IndexWriter.java:2253-2264`): `handleMergeException`
/// records every merge registered since `resetMergeExceptions`, because
/// `mergeGen` only moves there (`IndexWriter.java:3012-3015, 4996, 5600-5603`),
/// even when other merges are registered after it.
#[test]
fn an_aborted_forced_merge_surfaces_background_merge_hit_exception() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    {
        // Four one-document segments, written without merging.
        let mut config = IndexWriterConfig::new();
        config
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..4 {
            writer.add_document(&id_doc(i)).unwrap();
            writer.commit().unwrap();
        }
        writer.close().unwrap();
    }
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let cms = gated_cms(&gate, &started, &finished);
    let mut config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::APPEND).unwrap();
    config.set_merge_policy(log_doc_policy(2)).unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();

    // `forceMerge(1)` with merge factor 2 registers two merges of two
    // segments each (`LogMergePolicy.findForcedMergesMaxNumSegments`).
    let force_merger = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || writer.force_merge(1))
    };
    assert!(eventually(60, || started.load(Ordering::SeqCst) == 2));
    // Two more segments: their commit registers a natural merge after the
    // forced ones.
    for i in 4..6 {
        writer.add_document(&id_doc(i)).unwrap();
        writer.commit().unwrap();
    }
    assert!(eventually(60, || started.load(Ordering::SeqCst) == 3));

    // Roll back: every running merge is aborted, and the rollback waits for
    // them, so the gate opens once they all are.
    let rollback = {
        let writer = Arc::clone(&writer);
        thread::spawn(move || writer.rollback())
    };
    assert!(eventually(60, || {
        let threads = cms.merge_threads();
        threads.len() == 3 && threads.iter().all(|t| t.merge().is_aborted())
    }));
    gate.open();
    let outcome = watchdog(60, move || force_merger.join().unwrap());
    rollback.join().unwrap().unwrap();
    match outcome {
        Err(LuceneError::Io(error)) => assert!(
            error
                .to_string()
                .starts_with("background merge hit exception: "),
            "{error}"
        ),
        other => panic!("expected the background merge exception, got {other:?}"),
    }
    assert_eq!(finished.load(Ordering::SeqCst), 3);
}

/// `MergeRateLimiter.maybePause` (`MergeRateLimiter.java:115-170`) at a rate
/// of `0`: `(long) (1e9 * secondsToPause)` is `Long.MAX_VALUE` for a positive
/// byte count, and the `long` arithmetic that follows wraps back to a pause,
/// so the writer stops until the rate changes; for zero bytes the quotient is
/// NaN, the cast yields `0` and nothing pauses.
#[test]
fn a_stopped_rate_limiter_pauses_writes_until_the_rate_changes() {
    let limiter = Arc::new(MergeRateLimiter::new());
    limiter.set_mb_per_sec(0.0).unwrap();
    let paused = {
        let limiter = Arc::clone(&limiter);
        watchdog(10, move || limiter.pause(0))
    };
    assert_eq!(paused.unwrap(), 0, "pausing for no bytes does not pause");

    let (done_tx, done_rx) = mpsc::channel();
    let writer = {
        let limiter = Arc::clone(&limiter);
        thread::spawn(move || {
            let outcome = limiter.pause(1);
            let _ = done_tx.send(());
            outcome
        })
    };
    assert!(
        done_rx.recv_timeout(Duration::from_millis(600)).is_err(),
        "a stopped merge wrote"
    );
    limiter.set_mb_per_sec(f64::INFINITY).unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the merge resumed once the rate changed");
    let paused = writer.join().unwrap().unwrap();
    assert!(paused >= 500_000_000, "paused only {paused} ns");
    assert!(limiter.get_total_stopped_ns() >= 500_000_000);
}

/// `MergeRateLimiter.setMBPerSec(double)` rejects a negative rate with
/// `IllegalArgumentException("mbPerSec must be positive; got: " + mbPerSec)`
/// and leaves the rate as it was (`MergeRateLimiter.java:58-62`); `0.0` is
/// allowed and means the merge is paused.
#[test]
fn a_negative_merge_rate_is_rejected_as_lucene_does() {
    let limiter = MergeRateLimiter::new();
    limiter.set_mb_per_sec(5.0).unwrap();
    match limiter.set_mb_per_sec(-1.5) {
        Err(LuceneError::IllegalArgument(message)) => {
            assert_eq!(message, "mbPerSec must be positive; got: -1.5");
        }
        other => panic!("expected IllegalArgument, got {other:?}"),
    }
    assert_eq!(limiter.get_mb_per_sec(), 5.0);
    let as_trait: &dyn rucene::store::RateLimiter = &limiter;
    assert!(matches!(
        as_trait.set_mb_per_sec(-0.5),
        Err(LuceneError::IllegalArgument(_))
    ));
    limiter.set_mb_per_sec(0.0).unwrap();
    assert_eq!(limiter.get_mb_per_sec(), 0.0);
}

/// Asks for one merge of every segment, and only when a commit asks for
/// full-flush merges.
struct CommitOnlyMergePolicy;

impl std::fmt::Display for CommitOnlyMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CommitOnlyMergePolicy")
    }
}

impl MergePolicy for CommitOnlyMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_num_segments: i32,
        _segments_to_merge: &std::collections::HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let segments: Vec<_> = segment_infos
            .iter()
            .filter(|info| !merging.contains(&info.info.name))
            .cloned()
            .collect();
        if merge_trigger != MergeTrigger::Commit || segments.len() < 2 {
            return Ok(None);
        }
        let mut spec = MergeSpecification::new();
        spec.add(OneMerge::new(segments)?);
        Ok(Some(spec))
    }
}

/// `shutdown` closes through `rollbackInternal` once its commit succeeded
/// (`IndexWriter.java:1343`), whose deleter checkpoint and refresh
/// (`IndexWriter.java:2629-2633`) remove what a merge aborted after the final
/// commit wrote. Here the commit's own full-flush merge outlives
/// `maxFullFlushMergeWaitMillis`, leaves a partial file, and is aborted by the
/// close.
#[test]
fn close_removes_the_files_of_a_merge_aborted_after_the_final_commit() {
    ensure_codec();
    const PARTIAL: &str = "_zz.fdt";
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let started = Arc::new(AtomicBool::new(false));
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    {
        let (dir, started) = (Arc::clone(&dir), Arc::clone(&started));
        cms.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
            let probe = merge.clone();
            // The merge writes part of its output…
            let mut out = dir.create_output(PARTIAL, &*rucene::store::DEFAULT_IO_CONTEXT)?;
            out.write_bytes(b"partial", 0, 7)?;
            out.close()?;
            started.store(true, Ordering::SeqCst);
            // …and is still running when the close aborts it.
            while !probe.is_aborted() {
                thread::sleep(Duration::from_millis(5));
            }
            cms.default_do_merge(source, merge)
        })));
    }
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(CommitOnlyMergePolicy))
        .unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    config.set_max_full_flush_merge_wait_millis(50);
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.flush().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    // The close commits; the commit starts the full-flush merge, gives up on
    // it after 50 ms, and the close then aborts it.
    watchdog(60, move || writer.close()).unwrap();
    assert!(started.load(Ordering::SeqCst), "the full-flush merge ran");
    assert_eq!(committed_segment_count(&dir), 2);
    let files = dir.list_all().unwrap();
    assert!(
        !files.iter().any(|name| name == PARTIAL),
        "the aborted merge's file survived the close: {files:?}"
    );
}
