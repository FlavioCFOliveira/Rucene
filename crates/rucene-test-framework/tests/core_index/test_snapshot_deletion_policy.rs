//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSnapshotDeletionPolicy.java`.
//!
//! This was developed for Lucene In Action, <http://lucenebook.com>.
//!
//! `TestPersistentSnapshotDeletionPolicy` extends this class: the shared
//! members live in [`SnapshotFixture`] (the instance fields `snapshots` and
//! `buffer`, and the `protected` helpers), and the inherited test methods are
//! public so that the subclass binds them too.
//!
//! **Divergence.** Java's `PersistentSnapshotDeletionPolicy extends
//! SnapshotDeletionPolicy`, so the helpers taking a `SnapshotDeletionPolicy`
//! accept both; Rucene's persistent policy wraps the base one by composition
//! (declared in `src/index/index_deletion_policy.rs`), so the helpers take the
//! [`SnapshotPolicy`] trait, which both implement with their own (overriding)
//! `snapshot`/`release`, as Java's virtual dispatch does.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::{
    IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy, PersistentSnapshotDeletionPolicy,
    SnapshotDeletionPolicy,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::{Directory, READONCE_IO_CONTEXT};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::{assert_no_unreferenced_files, CountDownLatch};

/// `TestSnapshotDeletionPolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSnapshotDeletionPolicy",
        inherited_methods(),
    )
}

/// The `@Test` methods of `TestSnapshotDeletionPolicy`, which its subclass
/// `TestPersistentSnapshotDeletionPolicy` inherits.
pub fn inherited_methods() -> Vec<randomizedtesting::jdk::reflect::Method> {
    vec![
        test("testSnapshotDeletionPolicy", test_snapshot_deletion_policy),
        test("testBasicSnapshots", test_basic_snapshots),
        test(
            "testMultiThreadedSnapshotting",
            test_multi_threaded_snapshotting,
        ),
        test("testRollbackToOldSnapshot", test_rollback_to_old_snapshot),
        test("testReleaseSnapshot", test_release_snapshot),
        test(
            "testSnapshotLastCommitTwice",
            test_snapshot_last_commit_twice,
        ),
        test("testMissingCommits", test_missing_commits),
    ]
}

/// `SnapshotDeletionPolicy`'s public surface, which
/// `PersistentSnapshotDeletionPolicy` inherits and partly overrides (see the
/// module documentation).
pub trait SnapshotPolicy {
    /// `snapshot()`.
    fn snapshot(&self) -> Result<Arc<dyn IndexCommit>>;
    /// `release(IndexCommit)`.
    fn release(&self, commit: &dyn IndexCommit) -> Result<()>;
    /// `getSnapshots()`.
    fn get_snapshots(&self) -> Vec<Arc<dyn IndexCommit>>;
    /// `getSnapshotCount()`.
    fn get_snapshot_count(&self) -> i32;
    /// `getIndexCommit(long)`.
    fn get_index_commit(&self, gen: i64) -> Option<Arc<dyn IndexCommit>>;
}

impl SnapshotPolicy for SnapshotDeletionPolicy {
    fn snapshot(&self) -> Result<Arc<dyn IndexCommit>> {
        SnapshotDeletionPolicy::snapshot(self)
    }
    fn release(&self, commit: &dyn IndexCommit) -> Result<()> {
        SnapshotDeletionPolicy::release(self, commit)
    }
    fn get_snapshots(&self) -> Vec<Arc<dyn IndexCommit>> {
        SnapshotDeletionPolicy::get_snapshots(self)
    }
    fn get_snapshot_count(&self) -> i32 {
        SnapshotDeletionPolicy::get_snapshot_count(self)
    }
    fn get_index_commit(&self, gen: i64) -> Option<Arc<dyn IndexCommit>> {
        SnapshotDeletionPolicy::get_index_commit(self, gen)
    }
}

impl SnapshotPolicy for PersistentSnapshotDeletionPolicy {
    fn snapshot(&self) -> Result<Arc<dyn IndexCommit>> {
        PersistentSnapshotDeletionPolicy::snapshot(self)
    }
    fn release(&self, commit: &dyn IndexCommit) -> Result<()> {
        PersistentSnapshotDeletionPolicy::release(self, commit)
    }
    fn get_snapshots(&self) -> Vec<Arc<dyn IndexCommit>> {
        PersistentSnapshotDeletionPolicy::get_snapshots(self)
    }
    fn get_snapshot_count(&self) -> i32 {
        PersistentSnapshotDeletionPolicy::get_snapshot_count(self)
    }
    fn get_index_commit(&self, gen: i64) -> Option<Arc<dyn IndexCommit>> {
        PersistentSnapshotDeletionPolicy::get_index_commit(self, gen)
    }
}

/// `(SnapshotDeletionPolicy) policy`: the policy as the snapshot surface, or
/// `ClassCastException`.
pub fn as_snapshot_policy(policy: &dyn IndexDeletionPolicy) -> &dyn SnapshotPolicy {
    let any = policy.as_any();
    if let Some(p) = any.and_then(|a| a.downcast_ref::<PersistentSnapshotDeletionPolicy>()) {
        return p;
    }
    if let Some(p) = any.and_then(|a| a.downcast_ref::<SnapshotDeletionPolicy>()) {
        return p;
    }
    Throwable::new(
        "java.lang.ClassCastException",
        &format!(
            "{} cannot be cast to org.apache.lucene.index.SnapshotDeletionPolicy",
            policy.java_class_name()
        ),
    )
    .throw()
}

/// The instance state and `protected` helpers of `TestSnapshotDeletionPolicy`.
#[derive(Default)]
pub struct SnapshotFixture {
    /// `protected List<IndexCommit> snapshots = new ArrayList<>()`.
    pub snapshots: Vec<Arc<dyn IndexCommit>>,
}

/// `public static final String INDEX_PATH = "test.snapshots"`.
#[allow(dead_code)]
pub const INDEX_PATH: &str = "test.snapshots";

impl SnapshotFixture {
    /// `protected IndexWriterConfig getConfig(Random random, IndexDeletionPolicy dp)`.
    pub fn get_config(
        &self,
        random: &dyn Random,
        dp: Option<Arc<dyn IndexDeletionPolicy>>,
    ) -> Result<IndexWriterConfig> {
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(random),
        )));
        if let Some(dp) = dp {
            conf.set_index_deletion_policy(Some(dp))?;
        }
        Ok(conf)
    }

    /// `protected void checkSnapshotExists(Directory dir, IndexCommit c)`.
    pub fn check_snapshot_exists(&self, dir: &dyn Directory, c: &dyn IndexCommit) -> Result<()> {
        let seg_file_name = c.get_segments_file_name();
        Assert::assert_true_msg(
            &format!("segments file not found in directory: {seg_file_name}"),
            LuceneTestCase::slow_file_exists(dir, &seg_file_name)?,
        );
        Ok(())
    }

    /// `protected void checkMaxDoc(IndexCommit commit, int expectedMaxDoc)`.
    pub fn check_max_doc(&self, commit: &dyn IndexCommit, expected_max_doc: i32) -> Result<()> {
        let reader = directory_reader::open_with_commit(commit)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_equals(&expected_max_doc, &reader.max_doc());
        }));
        reader.close()?;
        if let Err(p) = result {
            std::panic::resume_unwind(p);
        }
        Ok(())
    }

    /// `protected void prepareIndexAndSnapshots(SnapshotDeletionPolicy sdp,
    /// IndexWriter writer, int numSnapshots)`.
    pub fn prepare_index_and_snapshots(
        &mut self,
        sdp: &dyn SnapshotPolicy,
        writer: &IndexWriter,
        num_snapshots: i32,
    ) -> Result<()> {
        for _i in 0..num_snapshots {
            // create dummy document to trigger commit.
            writer.add_document(&Document::new())?;
            writer.commit()?;
            self.snapshots.push(sdp.snapshot()?);
        }
        Ok(())
    }

    /// `protected SnapshotDeletionPolicy getDeletionPolicy()`.
    pub fn get_deletion_policy(&self) -> Result<Arc<SnapshotDeletionPolicy>> {
        Ok(Arc::new(SnapshotDeletionPolicy::new(Some(Arc::new(
            KeepOnlyLastCommitDeletionPolicy::new(),
        )))))
    }

    /// `protected void assertSnapshotExists(Directory dir, SnapshotDeletionPolicy
    /// sdp, int numSnapshots, boolean checkIndexCommitSame)`.
    pub fn assert_snapshot_exists(
        &self,
        dir: &dyn Directory,
        sdp: &dyn SnapshotPolicy,
        num_snapshots: i32,
        check_index_commit_same: bool,
    ) -> Result<()> {
        for i in 0..num_snapshots as usize {
            let snapshot = &self.snapshots[i];
            self.check_max_doc(&**snapshot, i as i32 + 1)?;
            self.check_snapshot_exists(dir, &**snapshot)?;
            let commit = sdp.get_index_commit(snapshot.get_generation());
            if check_index_commit_same {
                assert_same_commit(snapshot, commit.as_ref());
            } else {
                let commit = commit.unwrap_or_else(|| crate::support::npe("getIndexCommit"));
                assert_equals(&snapshot.get_generation(), &commit.get_generation());
            }
        }
        Ok(())
    }
}

/// `assertSame(expected, actual)` over commits: the same instance
/// (`Arc::ptr_eq`), JUnit's message otherwise.
pub fn assert_same_commit(expected: &Arc<dyn IndexCommit>, actual: Option<&Arc<dyn IndexCommit>>) {
    if !actual.is_some_and(|a| Arc::ptr_eq(expected, a)) {
        let actual = actual.map_or_else(|| "null".to_string(), |a| a.to_string());
        Assert::fail(&format!("expected same:<{expected}> was not:<{actual}>"))
    }
}

/// `(SnapshotDeletionPolicy) writer.getConfig().getIndexDeletionPolicy()`.
fn writer_sdp(writer: &IndexWriter) -> Arc<dyn IndexDeletionPolicy> {
    let dp = writer.get_config().index_deletion_policy();
    as_snapshot_policy(&*dp);
    dp
}

/// Port of `TestSnapshotDeletionPolicy.testSnapshotDeletionPolicy`.
pub fn test_snapshot_deletion_policy() -> Result<()> {
    let fs_dir = LuceneTestCase::new_directory();
    run_test(&*random(), &fs_dir.as_directory())?;
    fs_dir.close()
}

/// `private void runTest(Random random, Directory dir)`.
fn run_test(random: &dyn Random, dir: &Arc<dyn Directory>) -> Result<()> {
    let max_iterations = if *TEST_NIGHTLY { 100 } else { 10 };

    let fixture = SnapshotFixture::default();
    let dp = fixture.get_deletion_policy()?;
    let conf =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(random))));
    conf.set_index_deletion_policy(Some(Arc::clone(&dp) as Arc<dyn IndexDeletionPolicy>))?;
    conf.set_max_buffered_docs(2)?;
    let writer = IndexWriter::new(Arc::clone(dir), conf)?;

    // Verify we catch misuse:
    expect_throws("java.lang.IllegalStateException", || dp.snapshot());

    writer.commit()?;

    let t = {
        let writer = Arc::clone(&writer);
        Thread::unnamed(move || {
            let mut iterations = 0;
            let mut doc = Document::new();
            let custom_type = stored_tv_type().unwrap_or_else(|e| rethrow_java(e));
            doc.add(Box::new(LuceneTestCase::new_field(
                "content",
                "aaa",
                &custom_type,
            )));
            loop {
                for i in 0..27 {
                    if let Err(t) = writer.add_document(&doc) {
                        println!("{t:?}");
                        Assert::fail("addDocument failed");
                    }
                    if i % 2 == 0 {
                        if let Err(e) = writer.commit() {
                            rethrow_java(e);
                        }
                    }
                }
                Thread::sleep(1);
                iterations += 1;
                if iterations >= max_iterations {
                    break;
                }
            }
        })
    };

    t.start();

    // While the above indexing thread is running, take many
    // backups:
    let mut buffer = vec![0u8; 4096];
    loop {
        backup_index(&mut buffer, &**dir, &dp)?;
        Thread::sleep(20);
        if !t.is_alive() {
            break;
        }
    }

    if let Err(p) = t.join() {
        std::panic::resume_unwind(p);
    }

    // Add one more document to force writer to commit a
    // final segment, so deletion policy has a chance to
    // delete again:
    let mut doc = Document::new();
    let custom_type = stored_tv_type()?;
    doc.add(Box::new(LuceneTestCase::new_field(
        "content",
        "aaa",
        &custom_type,
    )));
    writer.add_document(&doc)?;

    // Make sure we don't have any leftover files in the
    // directory:
    writer.close()?;
    assert_no_unreferenced_files(dir, "some files were not deleted but should have been")
}

/// `new FieldType(TextField.TYPE_STORED)` with term vectors, positions and
/// offsets.
fn stored_tv_type() -> Result<FieldType> {
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    Ok(custom_type)
}

/// Example showing how to use the SnapshotDeletionPolicy to take a backup. This
/// method does not really do a backup; instead, it reads every byte of every
/// file just to test that the files indeed exist and are readable even while
/// the index is changing.
fn backup_index(buffer: &mut [u8], dir: &dyn Directory, dp: &SnapshotDeletionPolicy) -> Result<()> {
    // To backup an index we first take a snapshot:
    let snapshot = dp.snapshot()?;
    let result = copy_files(buffer, dir, &*snapshot);
    // Make sure to release the snapshot, otherwise these
    // files will never be deleted during this IndexWriter
    // session:
    let released = dp.release(&*snapshot);
    result?;
    released
}

/// `private void copyFiles(Directory dir, IndexCommit cp)`.
fn copy_files(buffer: &mut [u8], dir: &dyn Directory, cp: &dyn IndexCommit) -> Result<()> {
    // While we hold the snapshot, and nomatter how long
    // we take to do the backup, the IndexWriter will
    // never delete the files in the snapshot:
    let files: HashSet<String> = cp.get_file_names()?;
    for file_name in &files {
        // NOTE: in a real backup you would not use
        // readFile; you would need to use something else
        // that copies the file to a backup location.  This
        // could even be a spawned shell process (eg "tar",
        // "zip") that takes the list of files and builds a
        // backup.
        read_file(buffer, dir, file_name)?;
    }
    Ok(())
}

/// `private void readFile(Directory dir, String name)`.
fn read_file(buffer: &mut [u8], dir: &dyn Directory, name: &str) -> Result<()> {
    let mut input = dir.open_input(name, &*READONCE_IO_CONTEXT)?;
    let result = (|| -> Result<()> {
        let size = dir.file_length(name)?;
        let mut bytes_left = size;
        while bytes_left > 0 {
            let num_to_read = if bytes_left < buffer.len() as i64 {
                bytes_left as usize
            } else {
                buffer.len()
            };
            input.read_bytes_buffered(Some(buffer), 0, num_to_read, false)?;
            bytes_left -= num_to_read as i64;
        }
        // Don't do this in your real backups!  This is just
        // to force a backup to take a somewhat long time, to
        // make sure we are exercising the fact that the
        // IndexWriter should not delete this file even when I
        // take my time reading it.
        Thread::sleep(1);
        Ok(())
    })();
    let closed = input.close();
    result?;
    closed
}

/// Port of `TestSnapshotDeletionPolicy.testBasicSnapshots`.
pub fn test_basic_snapshots() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let num_snapshots = 3;

    // Create 3 snapshots: snapshot0, snapshot1, snapshot2
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(f.get_deletion_policy()? as _))?,
    )?;
    let dp = writer_sdp(&writer);
    let sdp = as_snapshot_policy(&*dp);
    f.prepare_index_and_snapshots(sdp, &writer, num_snapshots)?;
    writer.close()?;

    assert_equals(&(num_snapshots as usize), &sdp.get_snapshots().len());
    assert_equals(&num_snapshots, &sdp.get_snapshot_count());
    f.assert_snapshot_exists(&*dir.as_directory(), sdp, num_snapshots, true)?;

    // open a reader on a snapshot - should succeed.
    directory_reader::open_with_commit(&*f.snapshots[0])?.close()?;

    // open a new IndexWriter w/ no snapshots to keep and assert that all snapshots are gone.
    let sdp = f.get_deletion_policy()?;
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(sdp as _))?,
    )?;
    writer.delete_unused_files()?;
    writer.close()?;
    assert_equals_msg(
        Some("no snapshots should exist"),
        &1,
        &directory_reader::list_commits(dir.as_directory())?.len(),
    );
    dir.close()
}

/// Port of `TestSnapshotDeletionPolicy.testMultiThreadedSnapshotting`.
pub fn test_multi_threaded_snapshotting() -> Result<()> {
    let f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(f.get_deletion_policy()? as _))?,
    )?;
    let dp = writer_sdp(&writer);

    let num_threads = 10;
    let snapshots: Arc<std::sync::Mutex<Vec<Option<Arc<dyn IndexCommit>>>>> =
        Arc::new(std::sync::Mutex::new(vec![None; num_threads]));
    let starting_gun = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(num_threads);
    for i in 0..num_threads {
        let final_i = i;
        let (writer, dp, snapshots, starting_gun) = (
            Arc::clone(&writer),
            Arc::clone(&dp),
            Arc::clone(&snapshots),
            Arc::clone(&starting_gun),
        );
        let t = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();
                writer.add_document(&Document::new())?;
                writer.commit()?;
                let s = as_snapshot_policy(&*dp).snapshot()?;
                snapshots.lock().unwrap_or_else(|e| e.into_inner())[final_i] = Some(s);
                Ok(())
            };
            if let Err(e) = run() {
                // `throw new RuntimeException(e)`
                rethrow_java(e);
            }
        });
        t.set_name(format!("t{i}"));
        threads.push(t);
    }

    for t in &threads {
        t.start();
    }

    starting_gun.count_down();

    for t in &threads {
        if let Err(p) = t.join() {
            std::panic::resume_unwind(p);
        }
    }

    // Do one last commit, so that after we release all snapshots, we stay w/ one commit
    writer.add_document(&Document::new())?;
    writer.commit()?;

    let sdp = as_snapshot_policy(&*dp);
    for i in 0..num_threads {
        let s = snapshots.lock().unwrap_or_else(|e| e.into_inner())[i].clone();
        let s = s.unwrap_or_else(|| crate::support::npe("snapshots[i]"));
        sdp.release(&*s)?;
        writer.delete_unused_files()?;
    }
    assert_equals(
        &1,
        &directory_reader::list_commits(dir.as_directory())?.len(),
    );
    writer.close()?;
    dir.close()
}

/// Port of `TestSnapshotDeletionPolicy.testRollbackToOldSnapshot`.
pub fn test_rollback_to_old_snapshot() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let num_snapshots = 2;
    let dir = LuceneTestCase::new_directory();

    let sdp = f.get_deletion_policy()?;
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(Arc::clone(&sdp) as _))?,
    )?;
    f.prepare_index_and_snapshots(&*sdp, &writer, num_snapshots)?;
    writer.close()?;

    // now open the writer on "snapshot0" - make sure it succeeds
    let conf = f.get_config(&*random(), Some(Arc::clone(&sdp) as _))?;
    conf.set_index_commit(Some(Arc::clone(&f.snapshots[0])));
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    // this does the actual rollback
    writer.commit()?;
    writer.delete_unused_files()?;
    f.assert_snapshot_exists(&*dir.as_directory(), &*sdp, num_snapshots - 1, false)?;
    writer.close()?;

    // but 'snapshot1' files will still exist (need to release snapshot before they can be deleted).
    let seg_file_name = f.snapshots[1].get_segments_file_name();
    Assert::assert_true_msg(
        &format!("snapshot files should exist in the directory: {seg_file_name}"),
        LuceneTestCase::slow_file_exists(&*dir.as_directory(), &seg_file_name)?,
    );

    dir.close()
}

/// Port of `TestSnapshotDeletionPolicy.testReleaseSnapshot`.
pub fn test_release_snapshot() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(f.get_deletion_policy()? as _))?,
    )?;
    let dp = writer_sdp(&writer);
    let sdp = as_snapshot_policy(&*dp);
    f.prepare_index_and_snapshots(sdp, &writer, 1)?;

    // Create another commit - we must do that, because otherwise the "snapshot"
    // files will still remain in the index, since it's the last commit.
    writer.add_document(&Document::new())?;
    writer.commit()?;

    // Release
    let seg_file_name = f.snapshots[0].get_segments_file_name();
    sdp.release(&*f.snapshots[0])?;
    writer.delete_unused_files()?;
    writer.close()?;
    Assert::assert_false_msg(
        &format!("segments file should not be found in dirctory: {seg_file_name}"),
        LuceneTestCase::slow_file_exists(&*dir.as_directory(), &seg_file_name)?,
    );
    dir.close()
}

/// Port of `TestSnapshotDeletionPolicy.testSnapshotLastCommitTwice`.
pub fn test_snapshot_last_commit_twice() -> Result<()> {
    let f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_directory();

    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(f.get_deletion_policy()? as _))?,
    )?;
    let dp = writer_sdp(&writer);
    let sdp = as_snapshot_policy(&*dp);
    writer.add_document(&Document::new())?;
    writer.commit()?;

    let s1 = sdp.snapshot()?;
    let s2 = sdp.snapshot()?;
    assert_same_commit(&s1, Some(&s2)); // should be the same instance

    // create another commit
    writer.add_document(&Document::new())?;
    writer.commit()?;

    // release "s1" should not delete "s2"
    sdp.release(&*s1)?;
    writer.delete_unused_files()?;
    f.check_snapshot_exists(&*dir.as_directory(), &*s2)?;

    writer.close()?;
    dir.close()
}

/// Port of `TestSnapshotDeletionPolicy.testMissingCommits`.
pub fn test_missing_commits() -> Result<()> {
    let f = SnapshotFixture::default();
    // Tests the behavior of SDP when commits that are given at ctor are missing
    // on onInit().
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(f.get_deletion_policy()? as _))?,
    )?;
    let dp = writer_sdp(&writer);
    let sdp = as_snapshot_policy(&*dp);
    writer.add_document(&Document::new())?;
    writer.commit()?;
    let s1 = sdp.snapshot()?;

    // create another commit, not snapshotted.
    writer.add_document(&Document::new())?;
    writer.close()?;

    // open a new writer w/ KeepOnlyLastCommit policy, so it will delete "s1"
    // commit.
    IndexWriter::new(dir.as_directory(), f.get_config(&*random(), None)?)?.close()?;

    Assert::assert_false_msg(
        "snapshotted commit should not exist",
        LuceneTestCase::slow_file_exists(&*dir.as_directory(), &s1.get_segments_file_name())?,
    );
    dir.close()
}
