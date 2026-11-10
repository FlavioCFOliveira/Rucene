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
    OneMergeWrappingMergePolicy,
};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, NoMergeScheduler, SerialMergeScheduler,
};
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, SegmentInfos};
use rucene::store::{
    BufferedChecksumIndexInput, ByteBuffersDirectory, Directory, IOContext, IndexInput,
    IndexOutput, Lock,
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
    let ram: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    lines.push(format!(
        "committed_max_doc={}",
        infos.total_max_doc().unwrap()
    ));
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
    let ram: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    lines.push(format!(
        "committed_max_doc={}",
        infos.total_max_doc().unwrap()
    ));
    lines.push(format!("files={}", files(&ram)));
    assert_eq!(lines.join("\n"), run_java("auto-flush-failure").trim());
}

/// `flush-next-buffer-failure`: `flushNextBuffer()` fails on the refused
/// directory and closes the writer; the files the aborted per-thread writer
/// had already created stay behind, exactly the ones Java leaves.
#[test]
fn a_failed_flush_next_buffer_is_a_writer_tragedy_as_lucene_does() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::clone(&ram),
        armed: Arc::clone(&armed),
    });
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    fn warm(&self, _reader: &Arc<dyn LeafReader>) -> Result<()> {
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
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(CommitOnlyMergePolicy)))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    let policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(2).unwrap();
    config.set_merge_policy(Some(Arc::new(policy))).unwrap();
    config
        .set_merge_scheduler(Some(Arc::clone(&cms) as Arc<dyn MergeScheduler>))
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
        infos.total_max_doc().unwrap(),
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
    let field_type = FieldType::new();
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
    let ram: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(RefusingDirectory {
        inner: Arc::clone(&ram),
        extension,
        armed: Arc::clone(&armed),
    });
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let config = IndexWriterConfig::new();
            config
                .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    fn close(&self) -> Result<()> {
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
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
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
    lines.push(format!(
        "committed_max_doc={}",
        infos.total_max_doc().unwrap()
    ));
    writer.close().unwrap();
    assert_eq!(
        lines.join("\n"),
        run_java("add-indexes-release-failure").trim()
    );
}

/// A directory whose first `create_output` of a file ending in `suffix`, once
/// armed, waits at a gate until released and then fails: the merge thread
/// stops inside the compound-file or `.si` write while another thread runs.
struct GatedFailingDirectory {
    inner: Arc<dyn Directory>,
    suffix: &'static str,
    armed: AtomicBool,
    /// Set once the gated `create_output` was reached.
    entered: (Mutex<bool>, Condvar),
    /// Set to let the gated `create_output` fail.
    released: (Mutex<bool>, Condvar),
}

impl GatedFailingDirectory {
    fn new(suffix: &'static str) -> Self {
        Self {
            inner: Arc::new(ByteBuffersDirectory::new()),
            suffix,
            armed: AtomicBool::new(false),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }

    /// Waits until the gated `create_output` was reached.
    fn await_entered(&self) {
        let entered = self.entered.0.lock().unwrap();
        let (entered, timeout) = self
            .entered
            .1
            .wait_timeout_while(entered, Duration::from_secs(30), |entered| !*entered)
            .unwrap();
        assert!(
            *entered && !timeout.timed_out(),
            "the merge reached the gate"
        );
    }

    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}

impl Directory for GatedFailingDirectory {
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
        if self.armed.load(Ordering::SeqCst) && name.ends_with(self.suffix) {
            {
                let mut entered = self.entered.0.lock().unwrap();
                if !*entered {
                    *entered = true;
                    self.entered.1.notify_all();
                    drop(entered);
                    let released = self.released.0.lock().unwrap();
                    let _released = self
                        .released
                        .1
                        .wait_while(released, |released| !*released)
                        .unwrap();
                    return Err(LuceneError::Io(std::io::Error::other(format!(
                        "injected failure creating {name}"
                    ))));
                }
            }
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

/// Records every `IW` message and wakes whoever waits for one.
///
/// When [`abort_target`](Self::abort_target) holds a merge, every
/// `"now abort running merge "` message also renders that merge with
/// `OneMerge::seg_string()` — Java's `OneMerge.segString()` — before it is
/// recorded. `abortMerges` logs that message while it holds the writer's lock
/// (`IndexWriter.java:2788, 2806-2809`), and Java's `segString()` takes no
/// lock (`MergePolicy.java:349-368`), so an info stream may call it there.
#[derive(Default)]
struct IwLog {
    messages: Mutex<Vec<String>>,
    changed: Condvar,
    abort_target: Mutex<Option<OneMerge>>,
    rendered_on_abort: Mutex<Vec<String>>,
}

impl IwLog {
    /// Waits at most `timeout` for a message starting with `prefix`, and
    /// returns whether one arrived.
    fn await_message(&self, prefix: &str, timeout: Duration) -> bool {
        let messages = self.messages.lock().unwrap();
        let (messages, _) = self
            .changed
            .wait_timeout_while(messages, timeout, |messages| {
                !messages.iter().any(|message| message.starts_with(prefix))
            })
            .unwrap();
        messages.iter().any(|message| message.starts_with(prefix))
    }

    fn contains(&self, message: &str) -> bool {
        self.messages.lock().unwrap().iter().any(|m| m == message)
    }
}

impl InfoStream for IwLog {
    fn java_class_name(&self) -> &'static str {
        "IwLog"
    }
    fn message(&self, component: &str, message: &str) {
        if component == "IW" {
            if message.starts_with("now abort running merge ") {
                let target = self.abort_target.lock().unwrap().clone();
                if let Some(merge) = target {
                    let rendered = merge.seg_string();
                    self.rendered_on_abort.lock().unwrap().push(rendered);
                }
            }
            self.messages.lock().unwrap().push(message.to_string());
            self.changed.notify_all();
        }
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// What [`rollback_during_a_failing_merge_write`] observed.
struct RollbackDuringMerge {
    log: Arc<IwLog>,
    /// The writer's rendering of the merged segments, `segString()` of the
    /// whole index taken before the merge: both segments take part in it.
    merged_segments: String,
    merged: Result<()>,
    rolled_back: Result<()>,
    closed: bool,
}

impl RollbackDuringMerge {
    /// The checks both rollback tests share: the rollback succeeded and
    /// closed the writer, `abortMerges` logged the running merge as
    /// `segString(merge.segments)` (`IndexWriter.java:2806-2809`), and the
    /// info stream rendered the merge's `OneMerge::seg_string()` while the
    /// rollback held the writer's lock.
    fn assert_rolled_back(&self) {
        assert!(self.rolled_back.is_ok(), "{:?}", self.rolled_back);
        assert!(self.closed);
        let expected = format!("now abort running merge {}", self.merged_segments);
        assert!(
            self.log.contains(&expected),
            "{expected} not in {:?}",
            self.log.messages.lock().unwrap()
        );
        let rendered = self.log.rendered_on_abort.lock().unwrap();
        assert_eq!(rendered.len(), 1, "{rendered:?}");
        assert!(
            rendered[0].ends_with(" into _2 [maxNumSegments=1]"),
            "{rendered:?}"
        );
    }
}

/// Runs a forced merge whose write of the first file ending in `suffix`
/// stops at a gate, rolls the writer back from another thread while it
/// waits, then lets the write fail.
///
/// The rollback's `abortMerges` holds the writer's lock while it logs each
/// running merge (`IndexWriter.java:2788, 2806-2809`), and the info stream
/// renders that merge there with `OneMerge::seg_string()`: Java's
/// `OneMerge.segString()` takes no lock (`MergePolicy.java:349-368`), this
/// port's reads the merged segment through `OneMerge::info`'s mutex. A merge
/// thread that held that mutex while its failure deleted files under the
/// writer's lock deadlocked against the rollback; the merge is captured from
/// the policy by a `OneMergeWrappingMergePolicy`.
fn rollback_during_a_failing_merge_write(
    suffix: &'static str,
    no_cfs_ratio: f64,
) -> RollbackDuringMerge {
    ensure_codec();
    let dir = Arc::new(GatedFailingDirectory::new(suffix));
    let policy = rucene::index::merge_policy::TieredMergePolicy::new();
    policy.set_no_cfs_ratio(no_cfs_ratio).unwrap();
    let log = Arc::new(IwLog::default());
    let capturing = Arc::clone(&log);
    let policy = OneMergeWrappingMergePolicy::new(
        Arc::new(policy),
        Arc::new(move |merge: OneMerge| {
            *capturing.abort_target.lock().unwrap() = Some(merge.clone());
            merge
        }),
    );
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(Arc::new(policy))).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.commit().unwrap();
    let merged_segments = writer.seg_string().unwrap();
    assert!(merged_segments.starts_with("_0(") && merged_segments.contains(" _1("));
    dir.armed.store(true, Ordering::SeqCst);

    let (merged_tx, merged_rx) = mpsc::channel();
    let merging = Arc::clone(&writer);
    let merge_thread = thread::spawn(move || {
        let _ = merged_tx.send(merging.force_merge(1));
    });
    dir.await_entered();

    let (rolled_back_tx, rolled_back_rx) = mpsc::channel();
    let rolling_back = Arc::clone(&writer);
    let rollback_thread = thread::spawn(move || {
        let _ = rolled_back_tx.send(rolling_back.rollback());
    });
    // The rollback either reaches its wait for the running merge, or is held
    // rendering it until the write is released; two seconds is enough for the
    // former, and the latter must not deadlock once the write fails.
    let _ = log.await_message("now wait for", Duration::from_secs(2));
    dir.release();

    let merged = merged_rx.recv().unwrap();
    let rolled_back = rolled_back_rx.recv().unwrap();
    merge_thread.join().unwrap();
    rollback_thread.join().unwrap();
    let closed = writer.is_closed();
    RollbackDuringMerge {
        log,
        merged_segments,
        merged,
        rolled_back,
        closed,
    }
}

/// A rollback while a merge builds its compound file, which then fails: the
/// merge sees it was aborted and gives up (`IndexWriter.java:5481-5491`), and
/// the rollback completes. The compound file is built on a copy of the
/// merged segment, so the merge holds no lock the rollback needs while the
/// failure deletes the partial files under the writer's lock.
#[test]
fn a_rollback_during_a_failing_compound_file_write_does_not_deadlock() {
    let outcome = watchdog(60, || rollback_during_a_failing_merge_write(".cfs", 1.0));
    outcome.assert_rolled_back();
    assert!(outcome
        .log
        .contains("hit merge abort exception creating compound file during merge"));
}

/// A rollback while a merge writes its `.si`, which then fails: the merge
/// deletes its files and fails (`IndexWriter.java:5541-5551`), and the
/// rollback completes. The merged segment's files are read under the merge's
/// slot, which is released before they are deleted under the writer's lock.
#[test]
fn a_rollback_during_a_failing_segment_info_write_does_not_deadlock() {
    let outcome = watchdog(60, || rollback_during_a_failing_merge_write(".si", 0.0));
    outcome.assert_rolled_back();
    assert!(
        matches!(&outcome.merged, Err(LuceneError::Io(error)) if error.to_string() == "injected failure creating _2.si"),
        "{:?}",
        outcome.merged
    );
}

/// `abortMerges` logs each pending merge as `segString(merge.segments)`, the
/// writer's rendering of the merge's segments (`IndexWriter.java:2794-2797`),
/// not `OneMerge.segString()`: no `" [maxNumSegments=1]"`. A scheduler that
/// runs nothing leaves the forced merge pending until the rollback aborts it.
/// Measured against `lucene-core-10.5.0.jar` (`oracle6/Probe15.java`): `now
/// abort pending merge _0(10.5.0):c1 _1(10.5.0):c1`.
#[test]
fn a_rollback_logs_each_pending_merge_as_the_writer_renders_its_segments() {
    ensure_codec();
    let log = Arc::new(IwLog::default());
    let config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Some(Arc::new(NoMergeScheduler)))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.commit().unwrap();
    writer.force_merge_with(1, false).unwrap();
    writer.rollback().unwrap();
    let aborted: Vec<String> = log
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.starts_with("now abort pending merge "))
        .cloned()
        .collect();
    assert_eq!(
        aborted,
        vec!["now abort pending merge _0(10.5.0):c1 _1(10.5.0):c1".to_string()]
    );
}

/// `abortMerges` aborts the pending merges with `IOUtils.applyToAll`
/// (`IndexWriter.java:2792-2801`): a merge whose abort fails — here its
/// `mergeFinished` hook, which `abortOneMerge` runs through
/// `closeMergeReaders` (`IndexWriter.java:4911-4914, 5176-5209`) — does not
/// stop the others from being logged, aborted and finished; the first failure
/// is raised once all were tried, and `pendingMerges.clear()` is not reached.
/// Measured against `lucene-core-10.5.0.jar` (`oracle7/Probe17.java`, case
/// B1): with a `LogDocMergePolicy` of merge factor 2 over four committed
/// segments, a scheduler that runs nothing and no merge on commit, the
/// commits leave `_0 _1` and `_2 _3` pending; `rollback()` throws
/// `IOException: injected failure finishing _0,_1`, both merges are logged
/// and finished with `success=false`, `_0,_1` first, and afterwards
/// `hasPendingMerges()` is `true` and the writer is closed.
#[test]
fn a_rollback_aborts_every_pending_merge_and_raises_the_first_failure() {
    ensure_codec();
    let log = Arc::new(IwLog::default());
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let inner = LogMergePolicy::by_doc_count();
    inner.set_merge_factor(2).unwrap();
    let recorded = Arc::clone(&finished);
    let policy = OneMergeWrappingMergePolicy::new(
        Arc::new(inner),
        Arc::new(move |mut merge: OneMerge| {
            let recorded = Arc::clone(&recorded);
            merge.set_merge_finished(Arc::new(
                move |merge: &OneMerge,
                      success: bool,
                      _dropped: bool,
                      _deleter: Option<&mut rucene::index::IndexFileDeleter>| {
                    let names = merge
                        .segments
                        .iter()
                        .map(|segment| segment.info.name.clone())
                        .collect::<Vec<_>>()
                        .join(",");
                    recorded
                        .lock()
                        .unwrap()
                        .push(format!("{names} success={success}"));
                    if names == "_0,_1" {
                        return Err(LuceneError::Io(std::io::Error::other(format!(
                            "injected failure finishing {names}"
                        ))));
                    }
                    Ok(())
                },
            ));
            merge
        }),
    );
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(Arc::new(policy))).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(NoMergeScheduler)))
        .unwrap();
    config.set_max_full_flush_merge_wait_millis(0);
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for id in 0..4 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    let registered: Vec<String> = log
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.starts_with("add merge to pendingMerges: "))
        .cloned()
        .collect();
    assert_eq!(
        registered,
        vec![
            "add merge to pendingMerges: _0(10.5.0):c1 _1(10.5.0):c1 [total 1 pending]".to_string(),
            "add merge to pendingMerges: _2(10.5.0):c1 _3(10.5.0):c1 [total 2 pending]".to_string(),
        ]
    );
    assert!(writer.has_pending_merges_pub().unwrap());

    let rolled_back = writer.rollback();
    assert!(
        matches!(&rolled_back, Err(LuceneError::Io(error)) if error.to_string() == "injected failure finishing _0,_1"),
        "{rolled_back:?}"
    );
    let aborted: Vec<String> = log
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.starts_with("now abort pending merge "))
        .cloned()
        .collect();
    assert_eq!(
        aborted,
        vec![
            "now abort pending merge _0(10.5.0):c1 _1(10.5.0):c1".to_string(),
            "now abort pending merge _2(10.5.0):c1 _3(10.5.0):c1".to_string(),
        ]
    );
    assert_eq!(
        *finished.lock().unwrap(),
        vec![
            "_0,_1 success=false".to_string(),
            "_2,_3 success=false".to_string()
        ]
    );
    assert!(
        writer.has_pending_merges_pub().unwrap(),
        "the pending merges are not cleared"
    );
    assert!(writer.is_closed());
}

/// `handleMergeException` logs `segString(merge.segments)`, the writer's
/// rendering of the merged segments, not `OneMerge.segString()`
/// (`IndexWriter.java:4810-4813`): no `" into _2"`, no
/// `" [maxNumSegments=1]"`.
#[test]
fn handle_merge_exception_logs_the_writers_rendering_of_the_merged_segments() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(RefusingDirectory {
        inner: Arc::clone(&ram),
        extension: ".fdx",
        armed: Arc::clone(&armed),
    });
    let log = Arc::new(IwLog::default());
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.commit().unwrap();
    // Both segments take part in the merge, so the writer's rendering of the
    // index is its rendering of the merged segments. It is taken before the
    // merge: the failed merge is a tragedy that closes the writer, and
    // nothing changes the two segments in between.
    let merged_segments = writer.seg_string().unwrap();
    assert!(merged_segments.starts_with("_0(") && merged_segments.contains(" _1("));
    armed.store(true, Ordering::SeqCst);
    assert!(writer.force_merge(1).is_err());
    armed.store(false, Ordering::SeqCst);
    let expected = format!("handleMergeException: merge={merged_segments} exc=");
    let logged: Vec<String> = log
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|message| message.starts_with("handleMergeException: "))
        .cloned()
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].starts_with(&expected), "{logged:?} vs {expected}");
    writer.rollback().unwrap();
}

// ---------------------------------------------------------------------------
// Errors the merge path propagates (`WriterTragedyFixture` cases
// `pit-rollback`, `merge-close-failure`, `merge-middle-failure`,
// `register-failure` and `merging-after-rollback`)
// ---------------------------------------------------------------------------

/// `WriterTragedyFixture.hookedPolicy`: a `LogDocMergePolicy` of merge factor
/// `merge_factor` whose merges record `mergeFinished` in `finished` and fail it
/// when `fails` names the merge's first segment, or always for `"*"`; their
/// `wrapForMerge` fails when `fail_wrap`.
fn hooked_policy(
    finished: &Arc<Mutex<Vec<String>>>,
    fails: Option<&'static str>,
    fail_wrap: bool,
    merge_factor: i32,
) -> Arc<dyn MergePolicy> {
    let inner = LogMergePolicy::by_doc_count();
    inner.set_merge_factor(merge_factor).unwrap();
    let finished = Arc::clone(finished);
    Arc::new(OneMergeWrappingMergePolicy::new(
        Arc::new(inner),
        Arc::new(move |mut merge: OneMerge| {
            if fail_wrap {
                merge.set_wrap_for_merge(Arc::new(|_reader| {
                    Err(LuceneError::Io(std::io::Error::other(
                        "wrapForMerge failure",
                    )))
                }));
            }
            let finished = Arc::clone(&finished);
            merge.set_merge_finished(Arc::new(
                move |merge: &OneMerge,
                      success: bool,
                      _dropped: bool,
                      _deleter: Option<&mut rucene::index::IndexFileDeleter>| {
                    let names: Vec<String> = merge
                        .segments
                        .iter()
                        .map(|segment| segment.info.name.clone())
                        .collect();
                    let joined = names.join(",");
                    finished
                        .lock()
                        .unwrap()
                        .push(format!("{joined} success={success}"));
                    if fails.is_some_and(|fails| fails == "*" || fails == names[0]) {
                        return Err(LuceneError::Io(std::io::Error::other(format!(
                            "injected failure finishing {joined}"
                        ))));
                    }
                    Ok(())
                },
            ));
            merge
        }),
    ))
}

/// `java.util.List.toString()` of the finished merges.
fn finished_list(finished: &Arc<Mutex<Vec<String>>>) -> String {
    format!("[{}]", finished.lock().unwrap().join(", "))
}

/// `WriterTragedyFixture.sortedNames`: `getMergingSegments()`, each segment's
/// `toString()`, sorted.
fn merging_names(writer: &IndexWriter) -> String {
    let mut names: Vec<String> = writer
        .get_merging_segments_pub()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    names.sort();
    names.join(" ")
}

/// `pit-rollback`. With commit merges on, `preparePointInTimeMerge` wraps each
/// commit merge in a `OneMerge` whose `mergeFinished` does its own part and
/// then calls the wrapped merge's (`IndexWriter.java:3947-4008`). The policy's
/// failing hook must therefore still fail `rollback()`, which aborts the two
/// pending merges; and `getMergingSegments()` reports the merge whose abort
/// failed on the closed writer, as Java's does without `ensureOpen`
/// (`IndexWriter.java:2488-2490`).
#[test]
fn a_point_in_time_merge_calls_the_hook_of_the_merge_it_wraps() {
    ensure_codec();
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(hooked_policy(&finished, Some("_0"), false, 2)))
        .unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(NoMergeScheduler)))
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for id in 0..4 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    writer.force_merge_with(1, false).unwrap();
    let mut out = String::new();
    out.push_str(&format!(
        "pending-before={}\n",
        writer.has_pending_merges_pub().unwrap()
    ));
    out.push_str(&format!("rollback={}\n", described(&writer.rollback())));
    out.push_str(&format!("finished={}\n", finished_list(&finished)));
    out.push_str(&format!(
        "pending-after={}\n",
        writer.has_pending_merges_pub().unwrap()
    ));
    out.push_str(&format!("merging={}\n", merging_names(&writer)));
    out.push_str(&format!("open={}\n", writer.is_open()));
    assert_eq!(out, run_java("pit-rollback"));
}

/// A `SerialMergeScheduler` whose `wrapForMerge` fails, as the fixture's
/// anonymous subclass does.
struct WrapFailingScheduler(SerialMergeScheduler);

impl std::fmt::Display for WrapFailingScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WrapFailingScheduler")
    }
}

impl MergeScheduler for WrapFailingScheduler {
    fn merge(
        &self,
        merge_source: Arc<dyn rucene::index::merge_scheduler::MergeSource>,
        trigger: MergeTrigger,
    ) -> Result<()> {
        self.0.merge(merge_source, trigger)
    }

    fn wrap_for_merge(
        &self,
        _merge: &OneMerge,
        _directory: Arc<dyn Directory>,
    ) -> Result<Arc<dyn Directory>> {
        Err(LuceneError::illegal_state("wrap failure".to_string()))
    }

    fn same_thread_executor(&self) -> &Arc<rucene::util::SameThreadExecutorService> {
        self.0.same_thread_executor()
    }
}

/// The lines `WriterTragedyFixture.mergeFailure` prints, for a writer of three
/// committed one-document segments whose `forceMerge(1)` fails.
fn merge_failure(in_middle: bool, hook_throws: bool) -> String {
    ensure_codec();
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let config = IndexWriterConfig::new();
    config.set_max_full_flush_merge_wait_millis(0);
    if in_middle {
        config
            .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
    } else {
        config
            .set_merge_scheduler(Some(Arc::new(WrapFailingScheduler(
                SerialMergeScheduler::new(),
            ))))
            .unwrap();
    }
    config
        .set_merge_policy(Some(hooked_policy(
            &finished,
            hook_throws.then_some("*"),
            in_middle,
            10,
        )))
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for id in 0..3 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    let merging = Arc::clone(&writer);
    let force_merge = watchdog(60, move || described(&merging.force_merge(1)));
    let mut out = String::new();
    out.push_str(&format!("forceMerge={force_merge}\n"));
    out.push_str(&format!("finished={}\n", finished_list(&finished)));
    out.push_str(&format!("tragic={}\n", tragedy(&writer)));
    out.push_str(&format!("open={}\n", writer.is_open()));
    out
}

/// `merge-close-failure keep`: the scheduler's `wrapForMerge` fails before
/// `mergeMiddle`'s `try`, so `merge`'s `finally` is what closes the merge
/// readers (`IndexWriter.java:4866-4872`); the failure is the tragedy.
#[test]
fn a_merge_failing_before_its_readers_is_a_tragedy_as_lucene_does() {
    assert_eq!(
        merge_failure(false, false),
        run_java("merge-close-failure keep")
    );
}

/// `merge-close-failure throw`: closing the readers in `merge`'s `finally`
/// fails too, and that failure replaces the merge's own and becomes the
/// tragedy, as in Java. The failure leaves the `finally` before `mergeFinish`
/// (`IndexWriter.java:4869-4872`), so the merge stays in `runningMerges`; the
/// tragedy's rollback (`tragicEvent` at `IndexWriter.java:4890` →
/// `rollbackInternal`, `:2550`) then waits in `abortMerges` for the running
/// merges to finish (`:2816-2828`), on the very thread that would have to
/// finish this one, so `forceMerge` never returns — Lucene's behaviour, which
/// this port reproduces. The fixture reports `HUNG` after eight seconds.
///
/// The merge thread is left blocked for ever, as Java's is. It holds only
/// this writer and its `ByteBuffersDirectory`, whose lock factory is per directory,
/// so it keeps no lock another test needs; and a Rust test binary exits when
/// its tests are done, whatever other threads are still running.
#[test]
fn a_failure_closing_the_merge_readers_hangs_the_merge_thread_as_lucene_does() {
    let java = run_java("merge-close-failure throw");
    let mut java_lines: Vec<&str> = java.lines().collect();
    assert_eq!(java_lines.remove(0), "forceMerge=HUNG", "{java}");

    ensure_codec();
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::new(IwLog::default());
    let config = IndexWriterConfig::new();
    config.set_max_full_flush_merge_wait_millis(0);
    config
        .set_merge_scheduler(Some(Arc::new(WrapFailingScheduler(
            SerialMergeScheduler::new(),
        ))))
        .unwrap();
    config
        .set_merge_policy(Some(hooked_policy(&finished, Some("*"), false, 10)))
        .unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    for id in 0..3 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    let (merged_tx, merged_rx) = mpsc::channel();
    let merging = Arc::clone(&writer);
    // Deliberately detached: the thread blocks for ever, as Java's does.
    let _merge_thread = thread::spawn(move || {
        let _ = merged_tx.send(described(&merging.force_merge(1)));
    });
    // The tragedy's rollback waits for the running merge, logging each round
    // of `abortMerges`' loop (`IndexWriter.java:2816-2828`); two rounds,
    // each at least the one-second `doWait`, show it is looping, not passing
    // through.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let waiting = "now wait for 1 running merge/s to abort; currently running addIndexes: 0";
    loop {
        let rounds = log
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| *message == waiting)
            .count();
        if rounds >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the rollback never waited for the merge: {:?}",
            log.messages.lock().unwrap()
        );
        let messages = log.messages.lock().unwrap();
        let _ = log
            .changed
            .wait_timeout(messages, Duration::from_millis(200))
            .unwrap();
    }
    match merged_rx.recv_timeout(Duration::from_secs(2)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        other => panic!("forceMerge returned instead of hanging: {other:?}"),
    }
    let rust = format!(
        "finished={}\ntragic={}\nopen={}\n",
        finished_list(&finished),
        tragedy(&writer),
        writer.is_open()
    );
    assert_eq!(rust, format!("{}\n", java_lines.join("\n")));
}

/// `merge-middle-failure keep` and `throw`: a failure inside `mergeMiddle`'s
/// `try` closes the readers in its `finally` (`IndexWriter.java:5588-5594`);
/// a failure to close them replaces the merge's own failure and becomes the
/// tragedy.
#[test]
fn merge_middle_closes_its_readers_and_propagates_the_failure_as_lucene_does() {
    assert_eq!(
        merge_failure(true, false),
        run_java("merge-middle-failure keep")
    );
    assert_eq!(
        merge_failure(true, true),
        run_java("merge-middle-failure throw")
    );
}

/// Keeps the `IW` messages that start with one of its prefixes, as
/// `WriterTragedyFixture.recordingInfoStream` does.
struct PrefixLog {
    prefixes: &'static [&'static str],
    messages: Mutex<Vec<String>>,
}

impl InfoStream for PrefixLog {
    fn java_class_name(&self) -> &'static str {
        "PrefixLog"
    }
    fn message(&self, component: &str, message: &str) {
        if component == "IW" && self.prefixes.iter().any(|p| message.starts_with(p)) {
            self.messages.lock().unwrap().push(message.to_string());
        }
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// `register-failure`: `registerMerge` fails sizing a merge whose segment lost
/// a file (`IndexWriter.java:5022`), and `updatePendingMerges` propagates the
/// failure (`IndexWriter.java:2471-2477`). Lucene reports it as
/// `NoSuchFileException: _0.fdt`; this port's `ByteBuffersDirectory` raises an
/// `io::ErrorKind::NotFound` naming the same file.
///
/// The sizing comes after the merge was added to `pendingMerges`, logged, and
/// its segments added to `mergingSegments`, and before `registerDone`
/// (`IndexWriter.java:4984-5025`): the merge is pending and its segments
/// merging. `mergeFinish` only releases the segments of a registered merge
/// (`IndexWriter.java:5164-5170`), so the rollback's `assert
/// mergingSegments.isEmpty()` (`IndexWriter.java:2582-2583`) fails under
/// `-ea`, and this port's `debug_assert!` panics in a debug build; without
/// assertions both roll back cleanly. The oracle runs with `-ea` exactly when
/// this test is a debug build. The assertion renders the merging segments in
/// each implementation's own form, so only its text up to them is compared.
#[test]
fn a_failure_registering_a_merge_propagates_as_lucene_does() {
    ensure_codec();
    let java = run_java_matching_assertions("register-failure");
    let mut java_lines: Vec<&str> = java.lines().collect();
    assert_eq!(
        java_lines.remove(0),
        "maybeMerge=NoSuchFileException: _0.fdt cause=none",
        "{java}"
    );
    let log = Arc::new(PrefixLog {
        prefixes: &["add merge to pendingMerges", "registerMerge"],
        messages: Mutex::new(Vec::new()),
    });
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::new();
    config.set_max_full_flush_merge_wait_millis(0);
    config
        .set_merge_scheduler(Some(Arc::new(NoMergeScheduler)))
        .unwrap();
    config
        .set_merge_policy(Some(Arc::new(
            rucene::index::merge_policy::NoMergePolicy::new(),
        )))
        .unwrap();
    config.set_use_compound_file(false).unwrap();
    config
        .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for id in 0..2 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    dir.delete_file("_0.fdt").unwrap();
    let policy = LogMergePolicy::by_doc_count();
    policy.set_merge_factor(2).unwrap();
    let policy: Arc<dyn MergePolicy> = Arc::new(policy);
    let merged = writer.maybe_merge_with(&policy, MergeTrigger::Explicit, -1);
    assert!(
        matches!(&merged, Err(LuceneError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound && error.to_string().contains("_0.fdt")),
        "{merged:?}"
    );
    let mut rust_lines: Vec<String> = log
        .messages
        .lock()
        .unwrap()
        .iter()
        .map(|message| format!("IW {message}"))
        .collect();
    rust_lines.push(format!(
        "pending={}",
        writer.has_pending_merges_pub().unwrap()
    ));
    rust_lines.push(format!("merging={}", merging_names(&writer)));
    let java_rollback = java_lines.pop().expect("the rollback line");
    assert_eq!(rust_lines, java_lines, "{java}");

    let rolled_back = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
    match (rolled_back, cfg!(debug_assertions)) {
        (Ok(result), false) => {
            assert_eq!(format!("rollback={}", described(&result)), java_rollback);
        }
        (Err(panic), true) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_default();
            let prefix = "we aborted all merges but still have merging segments: ";
            assert!(message.starts_with(prefix), "{message}");
            assert!(
                java_rollback.starts_with(&format!("rollback=AssertionError: {prefix}")),
                "{java_rollback}"
            );
        }
        (Ok(result), true) => panic!("the debug assertion did not fire: {result:?}"),
        (Err(_), false) => panic!("the rollback panicked without debug assertions"),
    }
}

/// `merging-after-rollback`: the merge of `_0` and `_1` whose abort fails
/// keeps its segments merging, and `getMergingSegments()` reports both on the
/// closed writer even though the rollback restored a commit holding `_0`
/// only (`IndexWriter.java:2488-2490`).
#[test]
fn merging_segments_survive_a_rollback_as_lucene_reports_them() {
    ensure_codec();
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let config = IndexWriterConfig::new();
    config.set_max_full_flush_merge_wait_millis(0);
    config
        .set_merge_scheduler(Some(Arc::new(NoMergeScheduler)))
        .unwrap();
    config
        .set_merge_policy(Some(hooked_policy(&finished, Some("*"), false, 2)))
        .unwrap();
    let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
    writer.add_document(&id_doc(0)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&id_doc(1)).unwrap();
    writer.flush().unwrap();
    writer.add_document(&id_doc(2)).unwrap();
    writer.flush().unwrap();
    let mut out = String::new();
    out.push_str(&format!("merging-before={}\n", merging_names(&writer)));
    out.push_str(&format!("rollback={}\n", described(&writer.rollback())));
    out.push_str(&format!("open={}\n", writer.is_open()));
    out.push_str(&format!("merging-after={}\n", merging_names(&writer)));
    assert_eq!(out, run_java("merging-after-rollback"));
}

// ---------------------------------------------------------------------------
// Rollback during a merge, reader closes, and the hooks a wrapping merge chains
// ---------------------------------------------------------------------------

/// An input whose `close()` fails after closing the input it wraps, as the
/// anonymous `FilterIndexInput` of `WriterTragedyFixture.CloseFailingDirectory`
/// does.
struct CloseFailingInput {
    inner: Box<dyn IndexInput>,
}

impl rucene::store::DataInput for CloseFailingInput {
    /// `IndexInput.toString()` returns the resource description.
    fn to_java_string(&self) -> String {
        IndexInput::resource_description(self).to_string()
    }
    fn read_byte(&mut self) -> Result<u8> {
        self.inner.read_byte()
    }
    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.inner.read_bytes(b, offset, len)
    }
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.skip_bytes(num_bytes)
    }
}

impl IndexInput for CloseFailingInput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()?;
        Err(LuceneError::Io(std::io::Error::other(
            "injected failure closing an input",
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
    ) -> Result<Box<dyn IndexInput>> {
        self.inner.slice(slice_description, offset, length)
    }
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.inner.clone_input()
    }
    fn resource_description(&self) -> &str {
        self.inner.resource_description()
    }
}

/// The hook [`CloseFailingDirectory`] runs, once armed, when the first `.cfs`
/// is created.
type CfsHook = Box<dyn Fn() + Send + Sync>;

/// `WriterTragedyFixture.CloseFailingDirectory`: opens the input named
/// `failing_name` so that its `close()` fails, records the files deleted on
/// `recorded_thread`, and — the `cfs-rollback` subclass — once `cfs_armed`,
/// runs `cfs_hook` at the first `.cfs` it is asked for, starts recording the
/// thread that asked, and refuses the file.
struct CloseFailingDirectory {
    inner: Arc<dyn Directory>,
    failing_name: Mutex<Option<String>>,
    recorded_thread: Mutex<Option<thread::ThreadId>>,
    deleted_on_recorded_thread: Mutex<Vec<String>>,
    cfs_armed: AtomicBool,
    cfs_hook: Mutex<Option<CfsHook>>,
}

impl CloseFailingDirectory {
    fn new() -> Self {
        Self {
            inner: Arc::new(ByteBuffersDirectory::new()),
            failing_name: Mutex::new(None),
            recorded_thread: Mutex::new(None),
            deleted_on_recorded_thread: Mutex::new(Vec::new()),
            cfs_armed: AtomicBool::new(false),
            cfs_hook: Mutex::new(None),
        }
    }
}

impl Directory for CloseFailingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        if *self.recorded_thread.lock().unwrap() == Some(thread::current().id()) {
            self.deleted_on_recorded_thread
                .lock()
                .unwrap()
                .push(name.to_string());
        }
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if name.ends_with(".cfs")
            && self
                .cfs_armed
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            if let Some(hook) = self.cfs_hook.lock().unwrap().as_ref() {
                hook();
            }
            // Record what the merge thread deletes from here on.
            *self.recorded_thread.lock().unwrap() = Some(thread::current().id());
            return Err(LuceneError::Io(std::io::Error::other(format!(
                "injected failure creating {name}"
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
        let input = self.inner.open_input(name, context)?;
        if self.failing_name.lock().unwrap().as_deref() == Some(name) {
            return Ok(Box::new(CloseFailingInput { inner: input }));
        }
        Ok(input)
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

/// `WriterTragedyFixture.await(condition, what)`: waits up to thirty seconds
/// for `condition`, failing if it never holds.
fn await_condition(condition: impl Fn() -> bool, what: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !condition() {
        if std::time::Instant::now() > deadline {
            return Err(LuceneError::illegal_state(format!(
                "timed out waiting for {what}"
            )));
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// `WriterTragedyFixture.startRollback`: `writer.rollback()` on its own
/// thread, which sends its outcome.
fn start_rollback(writer: &Arc<IndexWriter>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let rolling_back = Arc::clone(writer);
    thread::spawn(move || {
        let _ = tx.send(described(&rolling_back.rollback()));
    });
    rx
}

/// The rollback's outcome, or `HUNG` if it did not finish within thirty
/// seconds, as the fixture's bounded `join` reports it.
fn rollback_outcome(rx: &Mutex<Option<mpsc::Receiver<String>>>) -> String {
    let rx = rx.lock().unwrap().take().expect("the rollback was started");
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| "HUNG".to_string())
}

/// `cfs-rollback`: another thread's `rollback()` aborts a merge while it
/// creates its compound file, which then fails. The merge returns 0, and the
/// `finally` of that step reports the failure and deletes the merged
/// segment's non-compound files on the merge thread (`IndexWriter.java:
/// 5480-5503`), before the rollback goes on.
#[test]
fn a_merge_aborted_while_building_its_compound_file_deletes_its_files_as_lucene_does() {
    let out = watchdog(120, || {
        ensure_codec();
        let dir = Arc::new(CloseFailingDirectory::new());
        let captured: Arc<Mutex<Option<OneMerge>>> = Arc::default();
        let messages = Arc::new(PrefixLog {
            prefixes: &[
                "hit merge abort exception creating compound file",
                "hit exception creating compound file",
            ],
            messages: Mutex::new(Vec::new()),
        });
        let config = IndexWriterConfig::new();
        config.set_max_full_flush_merge_wait_millis(0);
        config
            .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
        // Flushed segments without compound files: only the merge builds one.
        config.set_use_compound_file(false).unwrap();
        config
            .set_info_stream(Arc::clone(&messages) as Arc<dyn InfoStream>)
            .unwrap();
        let inner = LogMergePolicy::by_doc_count();
        inner.set_no_cfs_ratio(1.0).unwrap();
        let capturing = Arc::clone(&captured);
        config
            .set_merge_policy(Some(Arc::new(OneMergeWrappingMergePolicy::new(
                Arc::new(inner),
                Arc::new(move |merge: OneMerge| {
                    *capturing.lock().unwrap() = Some(merge.clone());
                    merge
                }),
            ))))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, config).unwrap();
        for id in 0..3 {
            writer.add_document(&id_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        let rollback: Arc<Mutex<Option<mpsc::Receiver<String>>>> = Arc::default();
        {
            let writer = Arc::clone(&writer);
            let captured = Arc::clone(&captured);
            let rollback = Arc::clone(&rollback);
            *dir.cfs_hook.lock().unwrap() = Some(Box::new(move || {
                *rollback.lock().unwrap() = Some(start_rollback(&writer));
                let merge = captured.lock().unwrap().clone().expect("the merge");
                await_condition(|| merge.is_aborted(), "the merge to abort").unwrap();
            }));
        }
        dir.cfs_armed.store(true, Ordering::SeqCst);
        let mut out = String::new();
        out.push_str(&format!(
            "forceMerge={}\n",
            described(&writer.force_merge(1))
        ));
        *dir.recorded_thread.lock().unwrap() = None;
        out.push_str(&format!("rollback={}\n", rollback_outcome(&rollback)));
        out.push_str(&format!(
            "messages=[{}]\n",
            messages.messages.lock().unwrap().join(", ")
        ));
        let mut deleted = dir.deleted_on_recorded_thread.lock().unwrap().clone();
        deleted.sort();
        out.push_str(&format!(
            "deleted-on-merge-thread-after-failure=[{}]\n",
            deleted.join(", ")
        ));
        out.push_str(&format!("open={}\n", writer.is_open()));
        // The hook holds the writer; release it.
        *dir.cfs_hook.lock().unwrap() = None;
        out
    });
    assert_eq!(out, run_java("cfs-rollback"));
}

/// `WriterTragedyFixture.cfsFailure`'s `InfoStream`: keeps the `IW` lines
/// that start with one of its prefixes, and stops recording the merge
/// thread's deletions at `merge`'s own `hit exception during merge`, before
/// the tragedy's rollback.
///
/// `handleMergeException` logs `t.toString()`, which for the `IOException`
/// the fixture's directory throws is `java.io.IOException: <message>`; this
/// port's `Display` of the [`LuceneError::Io`] it raises in its place is
/// `I/O error: <message>`, which is mapped to Java's.
struct CfsFailureLog {
    dir: Arc<CloseFailingDirectory>,
    messages: Mutex<Vec<String>>,
}

impl InfoStream for CfsFailureLog {
    fn java_class_name(&self) -> &'static str {
        "CfsFailureLog"
    }
    fn message(&self, component: &str, message: &str) {
        const PREFIXES: [&str; 4] = [
            "handleMergeException",
            "hit merge abort exception creating compound file",
            "hit exception creating compound file",
            "hit exception during merge",
        ];
        if message == "hit exception during merge" {
            *self.dir.recorded_thread.lock().unwrap() = None;
        }
        if component == "IW" && PREFIXES.iter().any(|p| message.starts_with(p)) {
            self.messages
                .lock()
                .unwrap()
                .push(message.replace(" exc=I/O error: ", " exc=java.io.IOException: "));
        }
    }
    fn is_enabled(&self, component: &str) -> bool {
        component == "IW"
    }
    fn close(&self) {}
}

/// `cfs-failure`: a merge cannot create its compound file, and nothing
/// aborted it. Under the writer's lock, `mergeMiddle`'s `catch` hands the
/// failure to `handleMergeException`, which logs it, sets it on the merge and
/// rethrows it; the `finally` then reports it and deletes the merged
/// segment's non-compound files (`IndexWriter.java:5481-5503`). `merge` hands
/// it to `handleMergeException` again, logs `hit exception during merge` and
/// records the tragedy (`IndexWriter.java:4850-4892`).
#[test]
fn a_compound_file_failure_is_handled_inside_merge_middle_as_lucene_does() {
    let out = watchdog(120, || {
        ensure_codec();
        let dir = Arc::new(CloseFailingDirectory::new());
        let log = Arc::new(CfsFailureLog {
            dir: Arc::clone(&dir),
            messages: Mutex::new(Vec::new()),
        });
        let config = IndexWriterConfig::new();
        config.set_max_full_flush_merge_wait_millis(0);
        config
            .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
        // Flushed segments without compound files: only the merge builds one.
        config.set_use_compound_file(false).unwrap();
        config
            .set_info_stream(Arc::clone(&log) as Arc<dyn InfoStream>)
            .unwrap();
        let policy = LogMergePolicy::by_doc_count();
        policy.set_no_cfs_ratio(1.0).unwrap();
        config.set_merge_policy(Some(Arc::new(policy))).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, config).unwrap();
        for id in 0..3 {
            writer.add_document(&id_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        dir.cfs_armed.store(true, Ordering::SeqCst);
        let mut out = String::new();
        out.push_str(&format!(
            "forceMerge={}\n",
            described(&writer.force_merge(1))
        ));
        *dir.recorded_thread.lock().unwrap() = None;
        out.push_str(&format!(
            "messages=[{}]\n",
            log.messages.lock().unwrap().join(", ")
        ));
        let mut deleted = dir.deleted_on_recorded_thread.lock().unwrap().clone();
        deleted.sort();
        out.push_str(&format!(
            "deleted-on-merge-thread-before-merge-finished=[{}]\n",
            deleted.join(", ")
        ));
        out.push_str(&format!("tragic={}\n", tragedy(&writer)));
        out.push_str(&format!("open={}\n", writer.is_open()));
        out
    });
    assert_eq!(out, run_java("cfs-failure"));
}

/// A merged-segment warmer that starts a rollback on another thread and waits
/// until the merge it warms is aborted, as `WriterTragedyFixture
/// .warmerRollback`'s does.
struct RollbackWarmer {
    writer: Mutex<Option<Arc<IndexWriter>>>,
    captured: Arc<Mutex<Option<OneMerge>>>,
    rollback: Arc<Mutex<Option<mpsc::Receiver<String>>>>,
}

impl std::fmt::Debug for RollbackWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RollbackWarmer")
    }
}

impl std::fmt::Display for RollbackWarmer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RollbackWarmer")
    }
}

impl MergedSegmentWarmer for RollbackWarmer {
    fn warm(&self, _reader: &Arc<dyn LeafReader>) -> Result<()> {
        let writer = self.writer.lock().unwrap().clone().expect("the writer");
        *self.rollback.lock().unwrap() = Some(start_rollback(&writer));
        let merge = self.captured.lock().unwrap().clone().expect("the merge");
        await_condition(|| merge.is_aborted(), "the merge to abort")
    }
}

/// `WriterTragedyFixture.warmerRollback(dropFails)`.
fn warmer_rollback(drop_fails: bool) -> String {
    ensure_codec();
    let dir = Arc::new(CloseFailingDirectory::new());
    let captured: Arc<Mutex<Option<OneMerge>>> = Arc::default();
    let rollback: Arc<Mutex<Option<mpsc::Receiver<String>>>> = Arc::default();
    let finished: Arc<Mutex<Vec<String>>> = Arc::default();
    let warmer = Arc::new(RollbackWarmer {
        writer: Mutex::new(None),
        captured: Arc::clone(&captured),
        rollback: Arc::clone(&rollback),
    });
    let config = IndexWriterConfig::new();
    config.set_max_full_flush_merge_wait_millis(0);
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config.set_use_compound_file(false).unwrap();
    config.set_reader_pooling(true);
    config
        .set_merged_segment_warmer(Some(Arc::clone(&warmer) as Arc<dyn MergedSegmentWarmer>))
        .unwrap();
    let inner = LogMergePolicy::by_doc_count();
    inner.set_no_cfs_ratio(0.0).unwrap();
    let capturing = Arc::clone(&captured);
    let recording = Arc::clone(&finished);
    config
        .set_merge_policy(Some(Arc::new(OneMergeWrappingMergePolicy::new(
            Arc::new(inner),
            Arc::new(move |mut merge: OneMerge| {
                let recording = Arc::clone(&recording);
                merge.set_merge_finished(Arc::new(
                    move |merge: &OneMerge,
                          success: bool,
                          _dropped: bool,
                          _deleter: Option<&mut rucene::index::IndexFileDeleter>| {
                        let names: Vec<String> = merge
                            .segments
                            .iter()
                            .map(|segment| segment.info.name.clone())
                            .collect();
                        recording
                            .lock()
                            .unwrap()
                            .push(format!("{} success={success}", names.join(",")));
                        Ok(())
                    },
                ));
                *capturing.lock().unwrap() = Some(merge.clone());
                merge
            }),
        ))))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, config).unwrap();
    *warmer.writer.lock().unwrap() = Some(Arc::clone(&writer));
    for id in 0..3 {
        writer.add_document(&id_doc(id)).unwrap();
        writer.commit().unwrap();
    }
    if drop_fails {
        // The stored-fields data of the merged segment, which the warmer's
        // pooled reader keeps open until `readerPool.drop` closes it.
        *dir.failing_name.lock().unwrap() = Some("_3.fdt".to_string());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "forceMerge={}\n",
        described(&writer.force_merge(1))
    ));
    out.push_str(&format!("rollback={}\n", rollback_outcome(&rollback)));
    out.push_str(&format!("finished={}\n", finished_list(&finished)));
    out.push_str(&format!("tragic={}\n", tragedy(&writer)));
    out.push_str(&format!("open={}\n", writer.is_open()));
    *warmer.writer.lock().unwrap() = None;
    out
}

/// `warmer-rollback keep`: a merge aborted while its merged segment is warmed
/// is skipped by `commitMerge`, which returns false (`IndexWriter.java:
/// 4692-4708`), and `mergeMiddle` returns 0; its `finally` still closes the
/// merge readers (`IndexWriter.java:5580-5594`), finishing the merge with
/// `success=false`.
#[test]
fn a_merge_aborted_after_warming_closes_its_readers_as_lucene_does() {
    let out = watchdog(120, || warmer_rollback(false));
    assert_eq!(out, run_java("warmer-rollback keep"));
}

/// `warmer-rollback drop-fails`: the skipped merge's pooled reader is dropped
/// by `readerPool.drop(merge.info)` (`IndexWriter.java:4705`), whose failure
/// to close the merged segment's input propagates out of `commitMerge` and
/// becomes the merge's tragedy.
#[test]
fn a_failure_dropping_an_aborted_merges_reader_propagates_as_lucene_does() {
    let out = watchdog(120, || warmer_rollback(true));
    assert_eq!(out, run_java("warmer-rollback drop-fails"));
}

/// A merge policy whose commit merges record their `onMergeComplete` and
/// `mergeFinished` hooks, as `PitHookProbe`'s does.
struct HookedCommitPolicy {
    events: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Display for HookedCommitPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HookedCommitPolicy")
    }
}

impl MergePolicy for HookedCommitPolicy {
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
        let mut merge = OneMerge::new(segments)?;
        let events = Arc::clone(&self.events);
        merge.set_on_merge_complete(Arc::new(move |merge: &OneMerge| {
            events
                .lock()
                .unwrap()
                .push(format!("onMergeComplete {}", merge.seg_string()));
            Ok(())
        }));
        let events = Arc::clone(&self.events);
        merge.set_merge_finished(Arc::new(
            move |merge: &OneMerge,
                  success: bool,
                  _dropped: bool,
                  _deleter: Option<&mut rucene::index::IndexFileDeleter>| {
                events.lock().unwrap().push(format!(
                    "mergeFinished {} success={success}",
                    merge.seg_string()
                ));
                Ok(())
            },
        ));
        let mut spec = MergeSpecification::new();
        spec.add(merge);
        Ok(Some(spec))
    }
}

/// A commit merge's own `onMergeComplete` still runs when
/// `preparePointInTimeMerge` wraps it: the wrapper does its part and then
/// calls `toWrap.onMergeComplete()` (`IndexWriter.java:4011-4024`).
///
/// `onMergeComplete` is package-private, so the oracle is a probe compiled
/// into `org.apache.lucene.index` (`PitHookProbe`), measured against
/// `lucene-core-10.5.0.jar`: a commit merge of `_0` and `_1` records
/// `[onMergeComplete _0(10.5.0):c1 _1(10.5.0):c1 into _2, mergeFinished
/// _0(10.5.0):c1 _1(10.5.0):c1 into _2 success=true]`, and the commit holds
/// `[_2(10.5.0):c2]`.
#[test]
fn a_point_in_time_merge_calls_the_on_merge_complete_of_the_merge_it_wraps() {
    let (events, committed) = watchdog(120, || {
        ensure_codec();
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
        config.set_max_full_flush_merge_wait_millis(30_000);
        config
            .set_merge_policy(Some(Arc::new(HookedCommitPolicy {
                events: Arc::clone(&events),
            })))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for id in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", id.to_string(), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        let events = format!("[{}]", events.lock().unwrap().join(", "));
        let committed: Vec<String> = SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        writer.close().unwrap();
        (events, format!("[{}]", committed.join(", ")))
    });
    assert_eq!(
        events,
        "[onMergeComplete _0(10.5.0):c1 _1(10.5.0):c1 into _2, \
         mergeFinished _0(10.5.0):c1 _1(10.5.0):c1 into _2 success=true]"
    );
    assert_eq!(committed, "[_2(10.5.0):c2]");
}

// ---------------------------------------------------------------------------
// The `catch` clauses around `termsHashPerField.add` (`IndexingChain.java:2001-2027`)
// ---------------------------------------------------------------------------

/// `ChainCatch.Toks`: emits `terms`, each carrying the term frequency `freq`
/// on its `PackedTokenAttributeImpl` — where Java's
/// `addAttribute(TermFrequencyAttribute.class)` lands under the default
/// attribute factory once `CharTermAttribute` is added.
#[derive(Debug)]
struct FreqTokens {
    source: rucene::util::AttributeSource,
    terms: Vec<&'static str>,
    freq: i32,
    upto: usize,
}

impl FreqTokens {
    fn new(freq: i32, terms: &[&'static str]) -> Self {
        let mut source = rucene::util::AttributeSource::new_with_factory(
            rucene::analysis::default_token_attribute_factory(),
        );
        source
            .add_attribute::<rucene::analysis::tokenattributes::PackedTokenAttributeImpl>()
            .unwrap();
        Self {
            source,
            terms: terms.to_vec(),
            freq,
            upto: 0,
        }
    }
}

impl rucene::analysis::TokenStream for FreqTokens {
    fn increment_token(&mut self) -> Result<bool> {
        use rucene::analysis::tokenattributes::{
            CharTermAttribute, PackedTokenAttributeImpl, TermFrequencyAttribute,
        };
        if self.upto == self.terms.len() {
            return Ok(false);
        }
        self.source.clear_attributes();
        let mut packed = self
            .source
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .unwrap();
        packed.append_string(Some(self.terms[self.upto]));
        packed.set_term_frequency(self.freq);
        self.upto += 1;
        Ok(true)
    }
    fn reset(&mut self) -> Result<()> {
        self.upto = 0;
        Ok(())
    }
    fn end(&mut self) -> Result<()> {
        self.source.end_attributes();
        Ok(())
    }
    fn attribute_source(&self) -> &rucene::util::AttributeSource {
        &self.source
    }
    fn attribute_source_mut(&mut self) -> &mut rucene::util::AttributeSource {
        &mut self.source
    }
}

fn chain_field_type(options: IndexOptions, tokenized: bool, vector_positions: bool) -> FieldType {
    let field_type = FieldType::new();
    field_type.set_tokenized(tokenized).unwrap();
    field_type.set_index_options(options).unwrap();
    if vector_positions {
        field_type.set_store_term_vectors(true).unwrap();
        field_type.set_store_term_vector_positions(true).unwrap();
    }
    field_type.freeze();
    field_type
}

fn freq_tokens_doc(freq: i32, terms: &[&'static str], field_type: FieldType) -> Document {
    let stream: std::sync::Arc<
        rucene::util::atomic_ref_cell::AtomicRefCell<dyn rucene::analysis::TokenStream>,
    > = std::sync::Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
        FreqTokens::new(freq, terms),
    ));
    let mut doc = Document::new();
    doc.add(Box::new(
        Field::new_with_token_stream("body", stream, field_type).unwrap(),
    ));
    doc
}

/// `Class: message`, the simple-name form of the line `ChainCatch` prints for
/// a throwable.
fn java_line(error: &LuceneError) -> String {
    let message = match error {
        LuceneError::IllegalArgument(message)
        | LuceneError::IllegalArgumentWithCause { message, .. }
        | LuceneError::IllegalState { message, .. }
        | LuceneError::AlreadyClosed { message, .. } => message.clone(),
        other => other.to_string(),
    };
    format!("{}: {message}", error.java_simple_name())
}

/// `ChainCatch.run`: one good document, then `bad`, then the writer's state
/// and a second good document. Returns `thrown`, `cause`, `isOpen`, `tragic`
/// and the second add's outcome (`numDocs/maxDoc` of an NRT reader, or the
/// error), as `ChainCatch` prints them.
fn chain_catch_case(bad: &Document) -> (String, String, bool, String, String) {
    ensure_codec();
    let writer = IndexWriter::new(
        Arc::new(ByteBuffersDirectory::new()),
        IndexWriterConfig::new(),
    )
    .unwrap();
    let mut ok = Document::new();
    ok.add(Box::new(
        StringField::new("id", "1".to_string(), Store::NO).unwrap(),
    ));
    writer.add_document(&ok).unwrap();
    let (thrown, cause) = match writer.add_document(bad) {
        Ok(_) => ("no exception".to_string(), "null".to_string()),
        Err(error) => {
            // The variants keep their cause as an `Arc<LuceneError>`.
            let cause = std::error::Error::source(&error)
                .and_then(|cause| cause.downcast_ref::<Arc<LuceneError>>())
                .map_or_else(|| "null".to_string(), |cause| java_line(cause));
            (java_line(&error), cause)
        }
    };
    let open = writer.is_open();
    let tragic = writer
        .get_tragic_exception()
        .map_or_else(|| "null".to_string(), |error| java_line(&error));
    let second = match writer.add_document(&ok) {
        Ok(_) => {
            let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
            let line = format!("numDocs={} maxDoc={}", reader.num_docs(), reader.max_doc());
            drop(reader);
            writer.close().unwrap();
            line
        }
        Err(error) => java_line(&error),
    };
    (thrown, cause, open, tragic, second)
}

/// `catch (Throwable th) { onAbortingException(th); throw th; }`
/// (`IndexingChain.java:2025-2027`): a failure of `termsHashPerField.add`
/// that is neither an over-long term nor a duplicate term aborts the segment,
/// and the writer closes with that very exception as its tragedy. Measured
/// against `lucene-core-10.5.0.jar` (`oracle/fix/ChainCatch.java`, cases A, B
/// and E, the same with and without `-ea`):
/// * A — a custom frequency of 2 with positions: `IllegalStateException:
///   field "body": cannot index positions while using custom
///   TermFrequencyAttribute` (`FreqProxTermsWriterPerField.getTermFreq`);
/// * B — a custom frequency of 2 on a docs-only field whose term repeats:
///   `IllegalStateException: field "body": must index term freq while using
///   custom TermFrequencyAttribute` (`addTerm`);
/// * E — a custom frequency of 2 on a freqs-only field storing term-vector
///   positions: `IllegalArgumentException: field "body": cannot index term
///   vector positions while using custom TermFrequencyAttribute`, which the
///   term-vectors consumer throws from inside `termsHashPerField.add`
///   (`nextPerField.add`).
///
/// Each time `isOpen=false`, the tragedy is the thrown exception, and the next
/// `addDocument` throws `AlreadyClosedException: this IndexWriter is closed`.
#[test]
fn a_failure_of_the_terms_hash_aborts_and_closes_the_writer_as_lucene_does() {
    let cases = [
        (
            "A",
            freq_tokens_doc(
                2,
                &["foo"],
                chain_field_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS, true, false),
            ),
            "IllegalStateException: field \"body\": cannot index positions while using custom TermFrequencyAttribute",
        ),
        (
            "B",
            freq_tokens_doc(
                2,
                &["foo", "foo"],
                chain_field_type(IndexOptions::DOCS, true, false),
            ),
            "IllegalStateException: field \"body\": must index term freq while using custom TermFrequencyAttribute",
        ),
        (
            "E",
            freq_tokens_doc(
                2,
                &["foo"],
                chain_field_type(IndexOptions::DOCS_AND_FREQS, true, true),
            ),
            "IllegalArgumentException: field \"body\": cannot index term vector positions while using custom TermFrequencyAttribute",
        ),
    ];
    for (name, doc, expected) in cases {
        let (thrown, cause, open, tragic, second) = chain_catch_case(&doc);
        assert_eq!(
            (
                thrown.as_str(),
                cause.as_str(),
                open,
                tragic.as_str(),
                second.as_str()
            ),
            (
                expected,
                "null",
                false,
                expected,
                "AlreadyClosedException: this IndexWriter is closed"
            ),
            "case {name}"
        );
    }
}

/// `catch (TermsHashPerField.DuplicateTermException e) { throw new
/// IllegalArgumentException("Document update skipped due to duplicate termdoc
/// term", e); }` (`IndexingChain.java:2022-2024`): a term a term-doc
/// (`DOCS_AND_CUSTOM_FREQS`) token-stream field repeats in one document is a
/// document-level failure; the cause carries the term
/// `TermsHashPerField.add` appends (`TermsHashPerField.java:199-204`).
/// Measured against `lucene-core-10.5.0.jar` (`oracle/fix/ChainCatch.java`,
/// case C): `IllegalArgumentException: Document update skipped due to
/// duplicate termdoc term`, cause `TermsHashPerField$DuplicateTermException:
/// field 'body' has duplicate term 'foo'` (an `IllegalStateException`),
/// `isOpen=true`, no tragedy, and the next document indexes:
/// `numDocs=2 maxDoc=2`.
#[test]
fn a_duplicate_term_doc_term_skips_only_the_document_as_lucene_does() {
    let doc = freq_tokens_doc(
        1,
        &["foo", "foo"],
        chain_field_type(IndexOptions::DOCS_AND_CUSTOM_FREQS, true, false),
    );
    assert_eq!(
        chain_catch_case(&doc),
        (
            "IllegalArgumentException: Document update skipped due to duplicate termdoc term"
                .to_string(),
            "IllegalStateException: field 'body' has duplicate term 'foo'".to_string(),
            true,
            "null".to_string(),
            "numDocs=2 maxDoc=2".to_string()
        )
    );
}

/// A field whose `invertableType()` is `BINARY`, wrapping a `Field` that holds
/// `foo` — `ChainCatch` case D2's anonymous `Field` subclass.
struct BinaryInvertedField(Field);

impl std::fmt::Display for BinaryInvertedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl rucene::index::IndexableField for BinaryInvertedField {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn field_type(&self) -> &dyn rucene::index::IndexableFieldType {
        self.0.field_type()
    }
    fn token_stream(
        &self,
        analyzer: Option<&dyn rucene::analysis::Analyzer>,
        reuse: &mut Option<Box<dyn rucene::analysis::TokenStream>>,
    ) -> Result<Box<dyn rucene::analysis::TokenStream>> {
        self.0.token_stream(analyzer, reuse)
    }
    fn binary_value(&self) -> Option<rucene::util::BytesRef> {
        self.0.binary_value()
    }
    fn string_value(&self) -> Option<String> {
        self.0.string_value()
    }
    fn reader_value(&self) -> Option<rucene::document::SharedReader> {
        None
    }
    fn numeric_value(&self) -> Option<rucene::document::NumericValue> {
        self.0.numeric_value()
    }
    fn stored_value(&self) -> Result<Option<rucene::document::StoredValue>> {
        self.0.stored_value()
    }
    fn invertable_type(&self) -> Option<rucene::document::InvertableType> {
        Some(rucene::document::InvertableType::BINARY)
    }
}

/// `invertTerm` catches only `MaxBytesLengthExceededException`
/// (`IndexingChain.java:2077-2096`), so the same duplicate term through a
/// `BINARY` field leaves as the `DuplicateTermException` itself and stays a
/// document-level failure. Measured against `lucene-core-10.5.0.jar`
/// (`oracle/fix/ChainCatch.java`, case D2): `TermsHashPerField$DuplicateTermException:
/// field 'body' has duplicate term 'foo'`, no cause, `isOpen=true`, no
/// tragedy, `numDocs=2 maxDoc=2`.
#[test]
fn a_duplicate_term_doc_term_of_a_binary_field_leaves_unwrapped_as_in_lucene() {
    let field_type = chain_field_type(IndexOptions::DOCS_AND_CUSTOM_FREQS, false, false);
    let mut doc = Document::new();
    for _ in 0..2 {
        doc.add(Box::new(BinaryInvertedField(
            Field::new_with_bytes(
                "body",
                rucene::util::BytesRef::new(b"foo".to_vec()),
                field_type.clone(),
            )
            .unwrap(),
        )));
    }
    assert_eq!(
        chain_catch_case(&doc),
        (
            "IllegalStateException: field 'body' has duplicate term 'foo'".to_string(),
            "null".to_string(),
            true,
            "null".to_string(),
            "numDocs=2 maxDoc=2".to_string()
        )
    );
}

/// `FieldInvertState.setAttributeSource` takes the frequency from
/// `addAttribute(TermFrequencyAttribute.class)` (`FieldInvertState.java:107`),
/// which under the default factory is the stream's `PackedTokenAttributeImpl`:
/// a frequency set there is indexed. Measured against
/// `lucene-core-10.5.0.jar` (`oracle/fix/ChainCatch.java`, case F, after the
/// audit's `PackedFreq.java`): `tfImpl=PackedTokenAttributeImpl`, then
/// `sumTotalTermFreq=3 freq=3 norm=3` for one token `foo` of frequency 3 on a
/// freqs-only field — the field length, and so the norm, count the frequency.
#[test]
fn a_term_frequency_on_the_packed_attribute_is_indexed_as_lucene_does() {
    ensure_codec();
    let writer = IndexWriter::new(
        Arc::new(ByteBuffersDirectory::new()),
        IndexWriterConfig::new(),
    )
    .unwrap();
    writer
        .add_document(&freq_tokens_doc(
            3,
            &["foo"],
            chain_field_type(IndexOptions::DOCS_AND_FREQS, true, false),
        ))
        .unwrap();
    let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    let leaf = leaves[0].leaf_reader();
    let sum_total_term_freq = leaf
        .terms("body")
        .unwrap()
        .unwrap()
        .sum_total_term_freq()
        .unwrap();
    let mut postings = leaf
        .postings(
            &Term::from_text("body", "foo"),
            rucene::index::POSTINGS_ENUM_FREQS,
        )
        .unwrap()
        .unwrap();
    assert_eq!(postings.next_doc().unwrap(), 0);
    let mut norms = leaf.get_norm_values("body").unwrap().unwrap();
    assert!(norms.advance_exact(0).unwrap());
    assert_eq!(
        (
            sum_total_term_freq,
            postings.freq().unwrap(),
            norms.long_value().unwrap()
        ),
        (3, 3, 3)
    );
    drop(leaves);
    drop(reader);
    writer.close().unwrap();
}

/// `startStoredFields` hands the throwable itself to `onAbortingException`
/// (`IndexingChain.java:563-576`), so a stored-fields file that cannot be
/// created closes the writer with that very `IOException` as its tragedy.
/// Measured against `lucene-core-10.5.0.jar` (`oracle/fix/StoredTragedy.java`,
/// the same with and without `-ea`): `thrown=java.io.IOException: disk full
/// creating _0.fdm`, `isOpen=false`, and the tragedy is the same exception.
#[test]
fn a_stored_fields_failure_is_the_tragedy_itself_as_in_lucene() {
    ensure_codec();
    let armed = Arc::new(AtomicBool::new(false));
    let dir: Arc<dyn Directory> = Arc::new(FailingDirectory {
        inner: Arc::new(ByteBuffersDirectory::new()),
        armed: Arc::clone(&armed),
    });
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    armed.store(true, Ordering::SeqCst);
    let mut doc = Document::new();
    doc.add(Box::new(
        rucene::document::StoredField::new_string("f", "v".to_string()).unwrap(),
    ));
    let thrown = writer.add_document(&doc);
    assert_eq!(exception(&thrown), "IOException: disk full creating _0.fdm");
    assert!(!writer.is_open());
    let tragic = writer.get_tragic_exception().expect("the writer is tragic");
    assert!(
        matches!(tragic.as_ref(), LuceneError::Io(error) if error.to_string() == "disk full creating _0.fdm"),
        "{tragic:?}"
    );
}
