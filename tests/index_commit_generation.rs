//! Regression tests for the commit generation of `IndexWriter`.
//!
//! Apache Lucene Core 10.5.0 names every commit point `segments_N`, with `N`
//! strictly increasing: `SegmentInfos.getNextPendingGeneration()` returns
//! `generation + 1` (or `1` when nothing has been committed yet),
//! `SegmentInfos.write(Directory)` writes `pending_segments_N` under that name,
//! and `SegmentInfos.finishCommit(Directory)` renames it to `segments_N`
//! (`SegmentInfos.java:526-598`, `:950-982` in the 10.5.0 tree).
//!
//! Rucene used to lose the generation between commits: `IndexWriter`'s snapshot
//! helper built the `toSync` copy with `SegmentInfos::replace`, which is Java's
//! `SegmentInfos.replace(SegmentInfos)` and deliberately does *not* copy
//! `generation`, instead of Java's `SegmentInfos.clone()`, which does. Every
//! commit therefore started again from generation `-1`, asked for generation
//! `1`, and tried to write `segments_1` a second time. These tests lock that
//! down, along with the two faults the same commit path carried:
//!
//! * a failing `rename` in `finishCommit` deleted the destination, which is the
//!   *previous* commit point, leaving the index with no commit at all;
//! * `close()` never released `write.lock`, so nothing else in the process
//!   could open the directory again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::document::{Document, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{NoDeletionPolicy, SegmentInfos, WRITE_LOCK_NAME};
use rucene::store::{Directory, IOContext, IndexOutput, Lock, RamDirectory};

/// The integration crate does not share the library's test-time codec registry,
/// so every entry point registers the default codec before writing an index.
fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
}

fn add_doc(writer: &IndexWriter, id: &str) {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", id.to_string(), Store::YES).unwrap(),
    ));
    writer.add_document(&doc).unwrap();
}

/// Every `segments_N` currently in `dir`, by generation, ascending.
fn commit_generations(dir: &dyn Directory) -> Vec<i64> {
    let mut generations: Vec<i64> = dir
        .list_all()
        .unwrap()
        .iter()
        .filter(|name| name.starts_with("segments_"))
        .map(|name| {
            SegmentInfos::generation_from_segments_file_name(name)
                .expect("a segments_N name parses as a generation")
        })
        .collect();
    generations.sort_unstable();
    generations
}

/// Names in `dir` that belong to an unfinished commit.
fn pending_commit_files(dir: &dyn Directory) -> Vec<String> {
    let mut names: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| name.starts_with("pending_segments_"))
        .collect();
    names.sort();
    names
}

#[test]
fn every_commit_writes_the_next_generation() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_index_deletion_policy(NoDeletionPolicy::instance())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();

    for generation in 1..=5i64 {
        add_doc(&writer, &format!("doc-{generation}"));
        writer.commit().unwrap();

        assert_eq!(
            SegmentInfos::get_last_commit_generation_dir(dir.as_ref()).unwrap(),
            generation,
            "commit {generation} must be the newest commit point"
        );
        assert_eq!(
            commit_generations(dir.as_ref()),
            (1..=generation).collect::<Vec<_>>(),
            "a retaining policy must keep every earlier segments_N"
        );
        assert!(
            pending_commit_files(dir.as_ref()).is_empty(),
            "a finished commit leaves no pending_segments_N behind"
        );
    }
}

#[test]
fn the_default_policy_keeps_only_the_newest_commit_but_still_advances() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    // `IndexWriterConfig::new()` installs `KeepOnlyLastCommitDeletionPolicy`,
    // the same default as Lucene 10.5.0.
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();

    for generation in 1..=4i64 {
        add_doc(&writer, &format!("doc-{generation}"));
        writer.commit().unwrap();

        assert_eq!(
            commit_generations(dir.as_ref()),
            vec![generation],
            "the default policy retains exactly the newest commit"
        );
    }
}

/// A directory whose `rename` fails once, on demand.
///
/// `FilterDirectory` forwards everything else, so the only behaviour under test
/// is what `SegmentInfos.finishCommit` does when the rename of
/// `pending_segments_N` to `segments_N` throws.
struct FailingRenameDirectory {
    inner: Arc<dyn Directory>,
    fail_rename: AtomicBool,
}

impl std::fmt::Debug for FailingRenameDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailingRenameDirectory")
            .field("fail_rename", &self.fail_rename)
            .finish_non_exhaustive()
    }
}

impl FailingRenameDirectory {
    fn new(delegate: Arc<dyn Directory>) -> Self {
        Self {
            inner: delegate,
            fail_rename: AtomicBool::new(false),
        }
    }
}

impl Directory for FailingRenameDirectory {
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
        if self.fail_rename.load(Ordering::Acquire) {
            return Err(LuceneError::Io(std::io::Error::other(format!(
                "injected rename failure: {source} -> {dest}"
            ))));
        }
        self.inner.rename(source, dest)
    }

    fn open_input(
        &self,
        name: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn rucene::store::IndexInput>> {
        self.inner.open_input(name, context)
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
}

#[test]
fn a_failed_rename_leaves_the_previous_commit_point_intact() {
    ensure_codec();
    let ram: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let dir = Arc::new(FailingRenameDirectory::new(Arc::clone(&ram)));
    let as_dir: Arc<dyn Directory> = Arc::clone(&dir) as Arc<dyn Directory>;

    let mut config = IndexWriterConfig::new();
    config
        .set_index_deletion_policy(NoDeletionPolicy::instance())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&as_dir), config).unwrap();

    add_doc(&writer, "first");
    writer.commit().unwrap();
    assert_eq!(commit_generations(as_dir.as_ref()), vec![1]);

    // The second commit cannot finish: `rename` throws.
    dir.fail_rename.store(true, Ordering::Release);
    add_doc(&writer, "second");
    let failed = writer.commit();
    assert!(failed.is_err(), "the injected rename failure must surface");

    // Java only deletes the destination once the rename has *succeeded* and the
    // directory sync then fails (`SegmentInfos.java:961-971`). A failing rename
    // must therefore leave `segments_1` — the previous, still valid commit
    // point — exactly where it is.
    assert_eq!(
        commit_generations(as_dir.as_ref()),
        vec![1],
        "a failed commit must not destroy the commit point that preceded it"
    );
    assert_eq!(
        SegmentInfos::get_last_commit_generation_dir(as_dir.as_ref()).unwrap(),
        1,
        "the index must still have a commit point after a failed commit"
    );
    // `rollbackCommit` removed the half-written pending file.
    assert!(
        pending_commit_files(as_dir.as_ref()).is_empty(),
        "a failed commit leaves no pending_segments_N behind"
    );
}

#[test]
fn closing_the_writer_releases_the_write_lock() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(
        rucene::store::FSDirectory::open(&{
            let path =
                std::env::temp_dir().join(format!("rucene-write-lock-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            path
        })
        .unwrap(),
    );

    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    add_doc(&writer, "only");
    assert!(
        dir.obtain_lock(WRITE_LOCK_NAME).is_err(),
        "an open writer must hold write.lock"
    );

    writer.close().unwrap();

    // Java closes `writeLock` and sets it to `null` inside
    // `IndexWriter.rollbackInternalNoCommit()`, so the directory is free as
    // soon as the writer is closed rather than whenever it is finally dropped.
    let mut reclaimed = dir
        .obtain_lock(WRITE_LOCK_NAME)
        .expect("close() must release write.lock");
    reclaimed.close().unwrap();
}
