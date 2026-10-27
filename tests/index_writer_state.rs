//! The state queries of `IndexWriter`, compared with Apache Lucene Core
//! 10.5.0 (Java oracle `WriterStateFixture`).
//!
//! * `has_uncommitted_changes` is `changeCount != lastCommitChangeCount ||
//!   hasChangesInRam()` (`IndexWriter.java:4207-4214`). In particular a writer
//!   opened with `OpenMode.CREATE` over an existing index has uncommitted
//!   changes from the start, because opening it called `changed()`
//!   (`IndexWriter.java:1022-1024`) — even though its segments still match the
//!   commit it replaces.
//! * `has_deletions` starts with `ensureOpen()` and reads the buffered
//!   packets, the documents writer, the reader pool and the segments
//!   (`IndexWriter.java:1457-1469`).
//!   A pooling writer whose NRT reader applied a delete has deletions that
//!   only its reader pool knows about.
//! * `setLiveCommitData(data)` is a committable change: it advances the
//!   change count and the `SegmentInfos` version; the two-argument overload
//!   with `false` advances only the change count (`IndexWriter.java:4138-4157`).
//! * `rollback` sets `closed` before it releases the write lock
//!   (`IndexWriter.java:2642-2645`), so whoever sees the lock released sees
//!   the writer closed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::error::LuceneError;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock, RamDirectory,
};

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
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

/// Runs `WriterStateFixture` and returns its standard output.
fn run_java() -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         WriterStateFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "WriterStateFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `WriterStateFixture.doc(int)`.
fn doc(id: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", id.to_string(), Store::YES).unwrap(),
    ));
    d
}

/// `WriterStateFixture.config()`.
fn config() -> IndexWriterConfig {
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config
}

fn term(text: &str) -> Term {
    Term::from_text("id", text)
}

#[test]
fn writer_state_queries_match_lucene() {
    ensure_codec();
    let mut lines = Vec::new();
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        w.add_document(&doc(0)).unwrap();
        w.commit().unwrap();
        w.close().unwrap();
        let mut create = config();
        create.set_open_mode(OpenMode::CREATE).unwrap();
        let w = IndexWriter::new(Arc::clone(&dir), create).unwrap();
        lines.push(format!("create={}", w.has_uncommitted_changes().unwrap()));
        w.rollback().unwrap();
    }
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        w.add_document(&doc(0)).unwrap();
        w.add_document(&doc(1)).unwrap();
        w.commit().unwrap();
        w.close().unwrap();
        let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        let state = |w: &IndexWriter, step: usize| {
            format!(
                " h{step}={} d{step}={}",
                w.has_uncommitted_changes().unwrap(),
                w.has_deletions().unwrap()
            )
        };
        let mut line = String::from("append");
        line.push_str(&state(&w, 0));
        w.delete_documents_by_terms(vec![term("0")]).unwrap();
        line.push_str(&state(&w, 1));
        drop(directory_reader::open_with_writer_flags(w.as_ref(), false, false).unwrap());
        line.push_str(&state(&w, 2));
        w.flush().unwrap();
        line.push_str(&state(&w, 3));
        w.commit().unwrap();
        line.push_str(&state(&w, 4));
        w.delete_documents_by_terms(vec![term("nomatch")]).unwrap();
        w.flush().unwrap();
        line.push_str(&state(&w, 5));
        lines.push(line);
        w.close().unwrap();
        let closed = match w.has_deletions() {
            Ok(_) => "ok",
            Err(LuceneError::AlreadyClosed(_)) => "AlreadyClosedException",
            Err(_) => "other",
        };
        lines.push(format!("closed={closed}"));
    }
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut pooling = config();
        pooling.set_reader_pooling(true);
        // No merge may fold the deletion away before the question is asked.
        pooling
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
        let w = IndexWriter::new(Arc::clone(&dir), pooling).unwrap();
        w.add_document(&doc(0)).unwrap();
        w.add_document(&doc(1)).unwrap();
        w.commit().unwrap();
        w.delete_documents_by_terms(vec![term("0")]).unwrap();
        let reader = directory_reader::open_with_writer(w.as_ref()).unwrap();
        lines.push(format!(
            "pooled={} numDocs={}",
            w.has_deletions().unwrap(),
            reader.num_docs()
        ));
        drop(reader);
        w.rollback().unwrap();
    }
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        w.add_document(&doc(0)).unwrap();
        w.add_document(&doc(1)).unwrap();
        w.commit().unwrap();
        w.add_document(&doc(2)).unwrap();
        w.flush().unwrap();
        w.delete_documents_by_terms(vec![term("0")]).unwrap();
        w.flush().unwrap();
        w.rollback().unwrap();
        let reopened = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        lines.push(format!(
            "reopen={}",
            reopened.has_uncommitted_changes().unwrap()
        ));
        reopened.close().unwrap();
    }
    {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
        w.add_document(&doc(0)).unwrap();
        w.commit().unwrap();
        let v1 = SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .unwrap()
            .version();
        let mut line = String::from("live");
        w.set_live_commit_data([("k".to_string(), "v".to_string())])
            .unwrap();
        line.push_str(&format!(" huc1={}", w.has_uncommitted_changes().unwrap()));
        w.commit().unwrap();
        let c2 = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        line.push_str(&format!(
            " data1={} dv1={}",
            c2.user_data(),
            c2.version() - v1
        ));
        line.push_str(&format!(" huc1c={}", w.has_uncommitted_changes().unwrap()));
        w.set_live_commit_data_with_increment([("k".to_string(), "w".to_string())], false)
            .unwrap();
        line.push_str(&format!(" huc2={}", w.has_uncommitted_changes().unwrap()));
        w.commit().unwrap();
        let c3 = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        line.push_str(&format!(
            " data2={} dv2={} gen={}",
            c3.user_data(),
            c3.version() - c2.version(),
            c3.generation()
        ));
        lines.push(line);
        w.close().unwrap();
    }
    assert_eq!(lines.join("\n"), run_java().trim());
}

/// Records, when the write lock is released, whether the writer that held it
/// already reported itself closed.
struct ObservingLock {
    inner: Box<dyn Lock>,
    writer: Arc<OnceLock<Weak<IndexWriter>>>,
    closed_at_release: Arc<Mutex<Option<bool>>>,
}

impl Lock for ObservingLock {
    fn close(&mut self) -> rucene::error::Result<()> {
        let closed = self
            .writer
            .get()
            .and_then(Weak::upgrade)
            .map(|writer| writer.is_closed());
        *self.closed_at_release.lock().unwrap() = closed;
        self.inner.close()
    }
    fn ensure_valid(&self) -> rucene::error::Result<()> {
        self.inner.ensure_valid()
    }
}

/// A directory whose locks are [`ObservingLock`]s.
struct ObservingDirectory {
    inner: Arc<dyn Directory>,
    writer: Arc<OnceLock<Weak<IndexWriter>>>,
    closed_at_release: Arc<Mutex<Option<bool>>>,
}

impl Directory for ObservingDirectory {
    fn list_all(&self) -> rucene::error::Result<Vec<String>> {
        self.inner.list_all()
    }
    fn delete_file(&self, name: &str) -> rucene::error::Result<()> {
        self.inner.delete_file(name)
    }
    fn file_length(&self, name: &str) -> rucene::error::Result<i64> {
        self.inner.file_length(name)
    }
    fn create_output(
        &self,
        name: &str,
        context: &dyn IOContext,
    ) -> rucene::error::Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> rucene::error::Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> rucene::error::Result<()> {
        self.inner.sync(names)
    }
    fn sync_metadata(&self) -> rucene::error::Result<()> {
        self.inner.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> rucene::error::Result<()> {
        self.inner.rename(source, dest)
    }
    fn open_input(
        &self,
        name: &str,
        context: &dyn IOContext,
    ) -> rucene::error::Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }
    fn open_checksum_input(
        &self,
        name: &str,
    ) -> rucene::error::Result<Box<BufferedChecksumIndexInput>> {
        self.inner.open_checksum_input(name)
    }
    fn obtain_lock(&self, name: &str) -> rucene::error::Result<Box<dyn Lock>> {
        Ok(Box::new(ObservingLock {
            inner: self.inner.obtain_lock(name)?,
            writer: Arc::clone(&self.writer),
            closed_at_release: Arc::clone(&self.closed_at_release),
        }))
    }
    fn close(&mut self) -> rucene::error::Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> rucene::error::Result<std::collections::HashSet<String>> {
        self.inner.get_pending_deletions()
    }
    fn ensure_open(&self) -> rucene::error::Result<()> {
        self.inner.ensure_open()
    }
}

/// `rollbackInternalNoCommit` sets `closed = true` and only then closes the
/// write lock (`IndexWriter.java:2642-2645`): a process that acquires the
/// released lock can never find the old writer still reporting itself open.
#[test]
fn rollback_closes_the_writer_before_it_releases_the_write_lock() {
    ensure_codec();
    let writer_slot = Arc::new(OnceLock::new());
    let closed_at_release = Arc::new(Mutex::new(None));
    let dir: Arc<dyn Directory> = Arc::new(ObservingDirectory {
        inner: Arc::new(RamDirectory::new()),
        writer: Arc::clone(&writer_slot),
        closed_at_release: Arc::clone(&closed_at_release),
    });
    let w = IndexWriter::new(Arc::clone(&dir), config()).unwrap();
    writer_slot.set(Arc::downgrade(&w)).unwrap();
    w.add_document(&doc(0)).unwrap();
    w.rollback().unwrap();
    assert_eq!(
        *closed_at_release.lock().unwrap(),
        Some(true),
        "the write lock was released while the writer still reported itself open"
    );
}
