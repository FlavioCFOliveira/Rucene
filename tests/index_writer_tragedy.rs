//! How `IndexWriter` reacts to tragedies and aborts, compared with Apache
//! Lucene Core 10.5.0.
//!
//! * `flush-failure` (Java oracle `WriterTragedyFixture flush-failure`): a
//!   failing flush is an aborting exception of the per-thread writer, which
//!   `DocumentsWriterPerThread.maybeAbort` reports through
//!   `FlushNotifications.onTragicEvent` (`DocumentsWriterPerThread.java:594-604`)
//!   — that is `IndexWriter.this.onTragicEvent` (`IndexWriter.java:446-447`) —
//!   and `doFlush`'s `maybeCloseOnTragicEvent` closes the writer
//!   (`IndexWriter.java:4415-4421`). The following commit fails with
//!   `AlreadyClosedException`.
//! * A tragedy hit by a point-in-time merge that runs on the committing thread
//!   closes the writer through `rollbackInternal`, which takes `commitLock`
//!   again on the thread that already holds it; Java's monitor is reentrant
//!   (`IndexWriter.java:2545-2549, 3862`), so the commit fails instead of
//!   hanging.
//! * `auto-flush-failure` and `flush-next-buffer-failure` (same oracle): the
//!   same tragedy reached through the flush `addDocument` triggers and through
//!   `flushNextBuffer()`, including the files the aborted flush leaves behind.
//! * `merge-index-failure` (same oracle): a merge whose fields index
//!   (`.fdx` or `.tvx`) cannot be created. `SegmentMerger` closes the
//!   stored-fields and term-vectors writers in try-with-resources
//!   (`SegmentMerger.java:246-267`), so `FieldsIndexWriter.close()` deletes
//!   its temporary files (`FieldsIndexWriter.java:183-201`) and the directory
//!   holds exactly the files Lucene leaves. `IndexWriter.handleMergeException`
//!   rethrows the merge's own exception (`IndexWriter.java:4808-4835`), so
//!   `forceMerge` raises the `IOException` and message the directory threw.
//! * `oversize-delete` (same oracle, over `commit`, `flush`, an NRT reader
//!   and `addIndexes(Directory...)`, which flushes first, run with `-ea` when this test is a debug build and without it
//!   otherwise): a 32767-byte delete term leaves the delete queue open, so the
//!   full flush cannot close it. Without assertions the call raises that
//!   `IllegalStateException` and the writer stays open; with them
//!   `finishFullFlush`'s `assert setFlushingDeleteQueue(null)`
//!   (`DocumentsWriter.java:645-651, 732`) is the writer's tragedy
//!   (`IndexWriter.java:713-715, 3858-3860, 4412-4414`), the close's own
//!   `pendingNumDocs` assertion (`IndexWriter.java:2545-2556`) is what the
//!   call raises when documents were buffered, and every later call fails
//!   with `AlreadyClosedException`.
//! * `add-indexes-release-failure` (same oracle): `addIndexes(Directory...)`
//!   succeeds but the source's write lock cannot be released; Java's
//!   `IOUtils.close(locks)` throws that failure (`IndexWriter.java:3205-3211`)
//!   and the writer keeps the copied segment.
//! * `deleteAll` aborts the running merges before it drops every segment
//!   (`IndexWriter.java:2726-2785`), so a merge of the deleted segments cannot
//!   bring their documents back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use rucene::codecs::Lucene104Codec;
use rucene::document::{Document, Field, FieldType, Store, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, MergedSegmentWarmer};
use rucene::index::leaf_reader::LeafReader;
use rucene::index::merge_policy::{
    LogMergePolicy, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, SerialMergeScheduler,
};
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, SegmentInfos};
use rucene::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock, RamDirectory,
};
use rucene::util::InfoStream;

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs one case of `WriterTragedyFixture` and returns its standard output.
/// `case` is the case name followed by its space-separated arguments.
fn run_java(case: &str) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         WriterTragedyFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(case.split(' '))
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "WriterTragedyFixture {case} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Runs `case` of `WriterTragedyFixture` with Java assertions enabled when
/// this test is a debug build, so the oracle runs Lucene's `assert`s exactly
/// when this port runs its `debug_assertions` checks.
fn run_java_matching_assertions(case: &str) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         WriterTragedyFixture.java",
    );
    let mut command = Command::new("java");
    if cfg!(debug_assertions) {
        command.arg("-ea");
    }
    let output = command
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(case.split(' '))
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "WriterTragedyFixture {case} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Runs `body` on its own thread and fails the test if it does not finish
/// within `seconds`, so a deadlock is reported instead of hanging the suite.
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
            Ok(()) => unreachable!("the body sent nothing yet returned"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("watchdog: deadlock after {seconds}s"),
    }
}

/// `WriterTragedyFixture.idDoc(int)`.
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

/// The class name `WriterTragedyFixture.outcome` prints for a Java throwable,
/// mapped from the error this port raises in its place.
fn outcome<T>(result: &Result<T>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(LuceneError::Io(_)) => "IOException",
        Err(LuceneError::AlreadyClosed { .. }) => "AlreadyClosedException",
        Err(LuceneError::IllegalState { .. }) => "IllegalStateException",
        Err(_) => "other",
    }
}

/// The `<SimpleName>: <message>` line `WriterTragedyFixture` prints for the
/// throwable a call raised, or `ok`, mapped from the error this port raises in
/// its place.
fn exception<T>(result: &Result<T>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(LuceneError::Io(error)) => format!("IOException: {error}"),
        // No other class is expected here; the variant is printed so a
        // mismatch shows what this port raised.
        Err(error) => format!("{error:?}"),
    }
}

/// `WriterTragedyFixture.FailingDirectory`: refuses to create any file but a
/// commit point while `armed`.
struct FailingDirectory {
    inner: Arc<dyn Directory>,
    armed: Arc<AtomicBool>,
}

impl Directory for FailingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if self.armed.load(Ordering::SeqCst)
            && !name.starts_with("segments")
            && !name.starts_with("pending")
        {
            return Err(LuceneError::Io(std::io::Error::other(format!(
                "disk full creating {name}"
            ))));
        }
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        if self.armed.load(Ordering::SeqCst) {
            return Err(LuceneError::Io(std::io::Error::other("disk full (temp)")));
        }
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
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.inner.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }
    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

/// `flush-failure`: the documents two acknowledged `addDocument` calls
/// buffered cannot be flushed; the writer records the tragedy, closes, and
/// refuses the following commit, so the last commit on disk still holds one
/// document.
#[test]
fn a_failed_flush_is_a_writer_tragedy_as_lucene_does() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.add_document(&id_doc(2)).unwrap();
    armed.store(true, Ordering::SeqCst);
    let flushed = writer.flush();
    armed.store(false, Ordering::SeqCst);
    let mut lines = vec![
        format!("flush={}", outcome(&flushed)),
        format!("tragic={}", writer.get_tragic_exception().is_some()),
        format!("open={}", writer.is_open()),
    ];
    let committed = writer.commit();
    lines.push(format!("commit={}", outcome(&committed)));
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&ram)).unwrap();
    lines.push(format!("committed_max_doc={}", infos.total_max_doc()));
    assert_eq!(lines.join("\n"), run_java("flush-failure").trim());
}

/// The files of `dir` but `write.lock`, sorted and comma-separated.
///
/// `WriterTragedyFixture.files(Directory)`.
fn files(dir: &Arc<dyn Directory>) -> String {
    let mut names: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| name != "write.lock")
        .collect();
    names.sort();
    names.join(",")
}

/// `auto-flush-failure`: the flush `addDocument` triggers once two documents
/// are buffered (`maxBufferedDocs=2`) is refused by the directory. The
/// aborting failure is the writer's tragedy, so every later call fails with
/// `AlreadyClosedException` and only the first commit is on disk.
#[test]
fn a_failed_auto_flush_is_a_writer_tragedy_as_lucene_does() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config.set_max_buffered_docs(2).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    armed.store(true, Ordering::SeqCst);
    let first = writer.add_document(&id_doc(1));
    let second = writer.add_document(&id_doc(2));
    armed.store(false, Ordering::SeqCst);
    let mut lines = vec![
        format!("add1={}", outcome(&first)),
        format!("add2={}", outcome(&second)),
        format!("tragic={}", writer.get_tragic_exception().is_some()),
        format!("open={}", writer.is_open()),
    ];
    lines.push(format!("commit={}", outcome(&writer.commit())));
    lines.push(format!(
        "add3={}",
        outcome(&writer.add_document(&id_doc(3)))
    ));
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&ram)).unwrap();
    lines.push(format!("committed_max_doc={}", infos.total_max_doc()));
    lines.push(format!("files={}", files(&ram)));
    assert_eq!(lines.join("\n"), run_java("auto-flush-failure").trim());
}

/// `flush-next-buffer-failure`: `flushNextBuffer()` fails on the refused
/// directory and closes the writer; the files the aborted per-thread writer
/// had already created stay behind, exactly the ones Java leaves.
#[test]
fn a_failed_flush_next_buffer_is_a_writer_tragedy_as_lucene_does() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    armed.store(true, Ordering::SeqCst);
    let flushed = writer.flush_next_buffer();
    armed.store(false, Ordering::SeqCst);
    let mut lines = vec![
        format!("flush_next_buffer={}", outcome(&flushed)),
        format!("tragic={}", writer.get_tragic_exception().is_some()),
        format!("open={}", writer.is_open()),
    ];
    lines.push(format!("rollback={}", outcome(&writer.rollback())));
    lines.push(format!("files={}", files(&ram)));
    assert_eq!(
        lines.join("\n"),
        run_java("flush-next-buffer-failure").trim()
    );
}

/// Records every `IW` message with the thread that wrote it.
#[derive(Default)]
struct ThreadLog(Mutex<Vec<(thread::ThreadId, String)>>);

impl InfoStream for ThreadLog {
    fn java_class_name(&self) -> &'static str {
        "ThreadLog"
    }
    fn message(&self, component: &str, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push((thread::current().id(), format!("{component}: {message}")));
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// A merged-segment warmer that always fails.
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

/// Asks for one merge of every segment not already merging, only from a
/// commit's full flush, so the commit itself runs a point-in-time merge.
struct CommitOnlyMergePolicy;

impl std::fmt::Display for CommitOnlyMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CommitOnlyMergePolicy")
    }
}

impl MergePolicy for CommitOnlyMergePolicy {
    fn find_merges(
        &self,
        _trigger: MergeTrigger,
        _infos: &SegmentInfos,
        _context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }
    fn find_forced_merges(
        &self,
        _infos: &SegmentInfos,
        _max_segment_count: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }
    fn find_forced_deletes_merges(
        &self,
        _infos: &SegmentInfos,
        _context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }
    fn find_full_flush_merges(
        &self,
        trigger: MergeTrigger,
        infos: &SegmentInfos,
        context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = context.get_merging_segments();
        let segments: Vec<_> = infos
            .iter()
            .filter(|info| !merging.contains(&info.info.name))
            .cloned()
            .collect();
        if trigger != MergeTrigger::Commit || segments.len() < 2 {
            return Ok(None);
        }
        let mut spec = MergeSpecification::new();
        spec.add(OneMerge::new(segments)?);
        Ok(Some(spec))
    }
}

/// A point-in-time merge that a commit runs on its own thread (serial merge
/// scheduler) fails in the merged-segment warmer: the tragedy closes the
/// writer through `rollbackInternal`, which takes `commitLock` again on the
/// committing thread (`IndexWriter.java:2545-2549, 3862`). The commit returns
/// the error, the writer is closed, and nothing hangs.
#[test]
fn a_tragedy_inside_commit_reenters_the_commit_lock() {
    ensure_codec();
    let log = Arc::new(ThreadLog::default());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(CommitOnlyMergePolicy))
        .unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config.set_reader_pooling(true);
    config.set_max_full_flush_merge_wait_millis(10_000);
    config
        .set_merged_segment_warmer(Some(Arc::new(FailingWarmer)))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    let committing = Arc::clone(&writer);
    let committed = watchdog(60, move || {
        let outcome = committing.commit().map_err(|e| e.to_string());
        (thread::current().id(), outcome)
    });
    let (commit_thread, committed) = committed;
    assert!(
        committed.is_err(),
        "the failed merge must fail the commit: {committed:?}"
    );
    assert!(writer.is_closed(), "the tragedy must close the writer");
    assert!(writer.get_tragic_exception().is_some());
    let messages = log.0.lock().unwrap();
    assert!(
        messages
            .iter()
            .any(|(thread, message)| *thread == commit_thread && message == "IW: rollback"),
        "the rollback must run on the thread holding the commit lock"
    );
}

/// A one-shot latch: threads wait until some thread opens it.
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

/// Records every `IW` message and opens `on_wait` when the writer starts
/// waiting for its running merges to abort.
///
/// `abortMerges` logs `now abort running merge` and marks each running merge
/// aborted, then logs `now wait for N running merge/s to abort` before it
/// waits (`IndexWriter.java:2806-2834`). Opening the merge's gate on the
/// second message lets it reach its next abort check only once it has been
/// aborted, with no timing involved.
struct AbortWatcher {
    messages: Mutex<Vec<String>>,
    on_wait: Arc<Gate>,
}

impl InfoStream for AbortWatcher {
    fn java_class_name(&self) -> &'static str {
        "AbortWatcher"
    }
    fn message(&self, component: &str, message: &str) {
        if message.starts_with("now wait for ") && message.contains("running merge/s to abort") {
            self.on_wait.open();
        }
        self.messages
            .lock()
            .unwrap()
            .push(format!("{component}: {message}"));
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// `deleteAll` while a merge of the segments it deletes is running: Java
/// aborts the merge (`IndexWriter.java:2733`) and waits for it to stop, so the
/// commit that follows holds no document. The test asserts the abort itself —
/// the `now abort running merge` message and the merge's aborted flag as the
/// merge thread sees it — not only its consequence on the index.
#[test]
fn delete_all_aborts_a_running_merge() {
    ensure_codec();
    let gate = Arc::new(Gate::default());
    let started = Arc::new(Gate::default());
    let aborted_seen: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
    let cms = Arc::new(ConcurrentMergeScheduler::new());
    {
        let (gate, started, aborted_seen) = (
            Arc::clone(&gate),
            Arc::clone(&started),
            Arc::clone(&aborted_seen),
        );
        cms.set_do_merge_override(Some(Arc::new(move |cms, source, merge| {
            started.open();
            gate.wait();
            *aborted_seen.lock().unwrap() = Some(merge.is_aborted());
            cms.default_do_merge(source, merge)
        })));
    }
    let log = Arc::new(AbortWatcher {
        messages: Mutex::new(Vec::new()),
        on_wait: Arc::clone(&gate),
    });
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    let mut policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(2).unwrap();
    config.set_merge_policy(Arc::new(policy)).unwrap();
    config
        .set_merge_scheduler(Arc::clone(&cms) as Arc<dyn MergeScheduler>)
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.commit().unwrap();
    let waiting = Arc::clone(&started);
    watchdog(60, move || waiting.wait());
    let deleting = Arc::clone(&writer);
    watchdog(60, move || deleting.delete_all()).unwrap();
    // Let the merge go on if `deleteAll` did not; opening twice is harmless.
    gate.open();
    let waiting = Arc::clone(&writer);
    watchdog(60, move || waiting.wait_for_merges()).unwrap();
    let messages = log.messages.lock().unwrap().clone();
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("IW: now abort running merge ")),
        "deleteAll must abort the running merge; IW messages: {messages:#?}"
    );
    assert_eq!(
        *aborted_seen.lock().unwrap(),
        Some(true),
        "the merge thread must see its merge aborted once deleteAll lets it go on"
    );
    writer.commit().unwrap();
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(
        infos.total_max_doc(),
        0,
        "a merge brought back the documents deleteAll dropped"
    );
    assert_eq!(writer.num_docs().unwrap(), 0);
    writer.close().unwrap();
}

/// `WriterTragedyFixture.RefusingDirectory`: refuses to create the one file
/// ending in `extension` while `armed`.
struct RefusingDirectory {
    inner: Arc<dyn Directory>,
    extension: &'static str,
    armed: Arc<AtomicBool>,
}

impl Directory for RefusingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if self.armed.load(Ordering::SeqCst) && name.ends_with(self.extension) {
            return Err(LuceneError::Io(std::io::Error::other(format!(
                "disk full creating {name}"
            ))));
        }
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
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
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.inner.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }
    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

/// `WriterTragedyFixture.TV_TYPE`: `TextField.TYPE_NOT_STORED` with term
/// vectors.
fn tv_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    field_type.set_store_term_vectors(true).unwrap();
    field_type.freeze();
    field_type
}

/// `WriterTragedyFixture.tvDoc(int)`.
fn tv_doc(id: i32) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", id.to_string(), Store::YES).unwrap(),
    ));
    doc.add(Box::new(
        Field::new("tv", "a b c".to_string(), tv_type()).unwrap(),
    ));
    doc
}

/// `merge-index-failure <extension>`: the merge cannot create its fields
/// index; the stored-fields or term-vectors writer is still closed, so its
/// `FieldsIndexWriter` temporary files are deleted and the directory holds
/// exactly what Lucene leaves.
fn merge_index_failure(extension: &'static str) {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(RefusingDirectory {
        inner: Arc::clone(&ram),
        extension,
        armed: Arc::clone(&armed),
    });
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&tv_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&tv_doc(1)).unwrap();
    writer.commit().unwrap();
    armed.store(true, Ordering::SeqCst);
    let merged = writer.force_merge(1);
    armed.store(false, Ordering::SeqCst);
    let lines = [
        format!("force_merge={}", exception(&merged)),
        format!("tragic={}", writer.get_tragic_exception().is_some()),
        format!("open={}", writer.is_open()),
        format!("files={}", files(&ram)),
    ];
    writer.rollback().unwrap();
    assert_eq!(
        lines.join("\n"),
        run_java(&format!("merge-index-failure {extension}")).trim()
    );
}

#[test]
fn a_merge_that_cannot_create_the_stored_fields_index_leaves_lucenes_files() {
    merge_index_failure(".fdx");
}

#[test]
fn a_merge_that_cannot_create_the_term_vectors_index_leaves_lucenes_files() {
    merge_index_failure(".tvx");
}

/// `WriterTragedyFixture.describe(Throwable)`: the `<SimpleName>: <message>`
/// of the Java throwable this port raises `error` in place of.
fn describe(error: &LuceneError) -> String {
    match error {
        LuceneError::IllegalArgument(message) if message.starts_with("bytes can be at most") => {
            format!("MaxBytesLengthExceededException: {message}")
        }
        LuceneError::IllegalArgument(message) => format!("IllegalArgumentException: {message}"),
        LuceneError::IllegalState { message, .. } => format!("IllegalStateException: {message}"),
        LuceneError::AlreadyClosed { message, .. } => format!("AlreadyClosedException: {message}"),
        LuceneError::AssertionError(message) => format!("AssertionError: {message}"),
        LuceneError::Io(error) => format!("IOException: {error}"),
        other => format!("{other:?}"),
    }
}

/// `WriterTragedyFixture.described(ThrowingRunnable)`: the error, then its
/// cause as [`std::error::Error::source`] returns it.
fn described<T>(result: &Result<T>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(error) => {
            // `Throwable.getCause()`: the error's own source, which for a
            // closed writer is its tragedy (`IndexWriter.java:917-921`).
            let cause = std::error::Error::source(error)
                .and_then(|source| source.downcast_ref::<Arc<LuceneError>>())
                .map_or_else(|| "none".to_string(), |cause| describe(cause));
            format!("{} cause={cause}", describe(error))
        }
    }
}

/// The writer's tragedy as `WriterTragedyFixture.describe` prints it, or
/// `none`: [`IndexWriter::get_tragic_exception`] returns the error itself, as
/// Java's `getTragicException()` returns the `Throwable`.
fn tragedy(writer: &IndexWriter) -> String {
    writer
        .get_tragic_exception()
        .map_or_else(|| "none".to_string(), |tragedy| describe(&tragedy))
}

/// `WriterTragedyFixture.oversizeDelete`'s `InfoStream`: keeps the
/// `hit tragic <SimpleName> inside <location>` lines
/// `IndexWriter.onTragicEvent` logs (`IndexWriter.java:5857-5860`).
#[derive(Default)]
struct TragicLog(Mutex<Vec<String>>);

impl InfoStream for TragicLog {
    fn java_class_name(&self) -> &'static str {
        "TragicLog"
    }
    fn message(&self, component: &str, message: &str) {
        if message.starts_with("hit tragic ") {
            self.0
                .lock()
                .unwrap()
                .push(format!("{component}: {message}"));
        }
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// `WriterTragedyFixture.committedIndex()`: a directory holding one committed
/// document, for `addIndexes(Directory...)`.
fn committed_index() -> Arc<dyn Directory> {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let source = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    source.add_document(&id_doc(100)).unwrap();
    source.close().unwrap();
    dir
}

/// `oversize-delete <op> <docs>`, run on `op` and `docs` across a grid: in a
/// debug build the failed assertion of the full flush is a tragedy that
/// closes the writer, as Java's `AssertionError` under `-ea`; in a release
/// build `op` fails with Java's `IllegalStateException` and the writer stays
/// usable.
#[test]
fn an_assertion_failure_in_a_full_flush_is_a_writer_tragedy_as_lucene_under_ea() {
    ensure_codec();
    for op in ["commit", "flush", "reader", "add-indexes"] {
        for docs in [0, 1, 3] {
            let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            let mut config = IndexWriterConfig::new();
            config
                .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
                .unwrap();
            let log = Arc::new(TragicLog::default());
            config
                .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
                .unwrap();
            let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
            for id in 0..docs {
                writer.add_document(&id_doc(id)).unwrap();
            }
            let big = Term::new("id", rucene::util::BytesRef::new(vec![b'x'; 32767]));
            let mut lines = vec![format!(
                "delete={}",
                described(&writer.delete_documents_by_terms(vec![big]))
            )];
            let run = |writer: &Arc<IndexWriter>| -> Result<()> {
                match op {
                    "commit" => writer.commit().map(|_| ()),
                    "flush" => writer.flush(),
                    "add-indexes" => writer.add_indexes_dirs(&[committed_index()]).map(|_| ()),
                    _ => directory_reader::open_with_writer(writer.as_ref())
                        .and_then(|reader| reader.close()),
                }
            };
            lines.push(format!("{op}1={}", described(&run(&writer))));
            lines.push(format!("{op}2={}", described(&run(&writer))));
            lines.push(format!(
                "add={}",
                described(&writer.add_document(&id_doc(docs)))
            ));
            lines.push(format!("tragic={}", tragedy(&writer)));
            lines.push(format!("open={}", writer.is_open()));
            lines.push(format!("files={}", files(&dir)));
            lines.push(format!(
                "tragic-messages={}",
                log.0.lock().unwrap().join(" | ")
            ));
            writer.rollback().unwrap();
            assert_eq!(
                lines.join("\n"),
                run_java_matching_assertions(&format!("oversize-delete {op} {docs}")).trim(),
                "{op} after {docs} documents"
            );
        }
    }
}

/// `WriterTragedyFixture.UnreleasableLockDirectory`: its write lock is
/// released and then reports a failure.
struct UnreleasableLockDirectory {
    inner: Arc<dyn Directory>,
}

/// The lock [`UnreleasableLockDirectory`] hands out.
struct UnreleasableLock {
    inner: Box<dyn Lock>,
    name: String,
}

impl Lock for UnreleasableLock {
    fn close(&mut self) -> Result<()> {
        self.inner.close()?;
        Err(LuceneError::Io(std::io::Error::other(format!(
            "cannot release {}",
            self.name
        ))))
    }
    fn ensure_valid(&self) -> Result<()> {
        self.inner.ensure_valid()
    }
}

impl Directory for UnreleasableLockDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
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
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.inner.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        Ok(Box::new(UnreleasableLock {
            inner: self.inner.obtain_lock(name)?,
            name: name.to_string(),
        }))
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }
    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

/// `add-indexes-release-failure`: the copy succeeds, so the failure to release
/// the source's write lock is what `addIndexes(Directory...)` raises.
#[test]
fn a_lock_release_failure_after_add_indexes_is_raised_as_lucene_does() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    let source: Arc<dyn Directory> = Arc::new(UnreleasableLockDirectory {
        inner: committed_index(),
    });
    let mut lines = vec![format!(
        "addIndexes={}",
        described(&writer.add_indexes_dirs(&[source]))
    )];
    lines.push(format!("open={}", writer.is_open()));
    lines.push(format!(
        "tragic={}",
        writer.get_tragic_exception().is_some()
    ));
    writer.commit().unwrap();
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    lines.push(format!("committed_max_doc={}", infos.total_max_doc()));
    writer.close().unwrap();
    assert_eq!(
        lines.join("\n"),
        run_java("add-indexes-release-failure").trim()
    );
}
