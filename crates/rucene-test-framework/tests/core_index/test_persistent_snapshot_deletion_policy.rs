//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPersistentSnapshotDeletionPolicy.java`.
//!
//! The class extends `TestSnapshotDeletionPolicy`: its inherited test methods
//! are bound here too, and its helpers come from
//! [`SnapshotFixture`](crate::test_snapshot_deletion_policy::SnapshotFixture).
//! Java's `setUp`/`tearDown` overrides only call `super`, so they are not
//! repeated.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::index_deletion_policy::{
    IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy, PersistentSnapshotDeletionPolicy,
    SNAPSHOTS_PREFIX,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::store::Directory;
use rucene_test_framework::store::mock_directory_wrapper::Failure;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test,
};
use crate::test_snapshot_deletion_policy::{inherited_methods, SnapshotFixture};

/// `TestPersistentSnapshotDeletionPolicy`.
pub fn class_def() -> Arc<Class> {
    let mut methods = inherited_methods();
    methods.extend([
        test("testExistingSnapshots", test_existing_snapshots),
        test("testNoSnapshotInfos", test_no_snapshot_infos),
        test("testMissingSnapshots", test_missing_snapshots),
        test("testExceptionDuringSave", test_exception_during_save),
        test("testSnapshotRelease", test_snapshot_release),
        test(
            "testSnapshotReleaseByGeneration",
            test_snapshot_release_by_generation,
        ),
    ]);
    class(
        "org.apache.lucene.index.TestPersistentSnapshotDeletionPolicy",
        methods,
    )
}

/// `private SnapshotDeletionPolicy getDeletionPolicy(Directory dir)`.
fn get_deletion_policy(dir: Arc<dyn Directory>) -> Result<Arc<PersistentSnapshotDeletionPolicy>> {
    Ok(Arc::new(PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        dir,
        OpenMode::CREATE,
    )?))
}

/// `(PersistentSnapshotDeletionPolicy) writer.getConfig().getIndexDeletionPolicy()`.
fn writer_psdp(writer: &IndexWriter) -> Arc<dyn IndexDeletionPolicy> {
    let dp = writer.get_config().index_deletion_policy();
    psdp_of(&*dp);
    dp
}

/// `(PersistentSnapshotDeletionPolicy) policy`.
fn psdp_of(dp: &dyn IndexDeletionPolicy) -> &PersistentSnapshotDeletionPolicy {
    match dp
        .as_any()
        .and_then(|a| a.downcast_ref::<PersistentSnapshotDeletionPolicy>())
    {
        Some(p) => p,
        None => randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            format!(
                "{} cannot be cast to org.apache.lucene.index.PersistentSnapshotDeletionPolicy",
                dp.java_class_name()
            ),
        )
        .throw(),
    }
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testExistingSnapshots`.
fn test_existing_snapshots() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let num_snapshots = 3;
    let dir = LuceneTestCase::new_mock_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(
            &*random(),
            Some(get_deletion_policy(dir.as_directory())? as _),
        )?,
    )?;
    let dp = writer_psdp(&writer);
    let psdp = psdp_of(&*dp);
    assert_null(&psdp.get_last_save_file());
    f.prepare_index_and_snapshots(psdp, &writer, num_snapshots)?;
    assert_not_null(&psdp.get_last_save_file());
    writer.close()?;

    // Make sure only 1 save file exists:
    let mut count = 0;
    for file in dir.list_all()? {
        if file.starts_with(SNAPSHOTS_PREFIX) {
            count += 1;
        }
    }
    assert_equals(&1, &count);

    // Make sure we fsync:
    dir.crash()?;
    dir.clear_crash();

    // Re-initialize and verify snapshots were persisted
    let psdp = Arc::new(PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        dir.as_directory(),
        OpenMode::APPEND,
    )?);

    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(&*random(), Some(psdp as _))?,
    )?;
    let dp = writer_psdp(&writer);
    let psdp = psdp_of(&*dp);

    assert_equals(&(num_snapshots as usize), &psdp.get_snapshots().len());
    assert_equals(&num_snapshots, &psdp.get_snapshot_count());
    f.assert_snapshot_exists(&*dir.as_directory(), psdp, num_snapshots, false)?;

    writer.add_document(&Document::new())?;
    writer.commit()?;
    f.snapshots.push(psdp.snapshot()?);
    assert_equals(&(num_snapshots as usize + 1), &psdp.get_snapshots().len());
    assert_equals(&(num_snapshots + 1), &psdp.get_snapshot_count());
    f.assert_snapshot_exists(&*dir.as_directory(), psdp, num_snapshots + 1, false)?;

    writer.close()?;
    drop(dp);
    drop(writer);
    dir.close()
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testNoSnapshotInfos`.
fn test_no_snapshot_infos() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        dir.as_directory(),
        OpenMode::CREATE,
    )?;
    dir.close()
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testMissingSnapshots`.
fn test_missing_snapshots() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    expect_throws("java.lang.IllegalStateException", || {
        PersistentSnapshotDeletionPolicy::with_open_mode(
            Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            dir.as_directory(),
            OpenMode::APPEND,
        )
    });

    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of `testExceptionDuringSave`.
struct FailOnPersist {
    do_fail: AtomicBool,
}

impl Failure for FailOnPersist {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if LuceneTestCase::call_stack_contains_method::<PersistentSnapshotDeletionPolicy>("persist")
        {
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("now fail on purpose"),
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testExceptionDuringSave`.
fn test_exception_during_save() -> Result<()> {
    let f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_mock_directory();
    dir.fail_on(Arc::new(FailOnPersist {
        do_fail: AtomicBool::new(false),
    }));
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(
            &*random(),
            Some(Arc::new(PersistentSnapshotDeletionPolicy::with_open_mode(
                Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
                dir.as_directory(),
                OpenMode::CREATE_OR_APPEND,
            )?) as _),
        )?,
    )?;
    writer.add_document(&Document::new())?;
    writer.commit()?;

    let dp = writer_psdp(&writer);
    let psdp = psdp_of(&*dp);
    match psdp.snapshot() {
        Ok(_) => {}
        Err(ioe) if java_throwable(&ioe).is_instance("java.io.IOException") => {
            if java_throwable(&ioe).message() == Some("now fail on purpose") {
                // ok
            } else {
                return Err(ioe);
            }
        }
        Err(e) => return Err(e),
    }
    assert_equals(&0, &psdp.get_snapshot_count());
    writer.close()?;
    assert_equals(
        &1,
        &directory_reader::list_commits(dir.as_directory())?.len(),
    );
    drop(dp);
    drop(writer);
    dir.close()
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testSnapshotRelease`.
fn test_snapshot_release() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(
            &*random(),
            Some(get_deletion_policy(dir.as_directory())? as _),
        )?,
    )?;
    let dp = writer_psdp(&writer);
    let psdp = psdp_of(&*dp);
    f.prepare_index_and_snapshots(psdp, &writer, 1)?;
    writer.close()?;

    psdp.release(&*f.snapshots[0])?;

    let psdp = PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        dir.as_directory(),
        OpenMode::APPEND,
    )?;
    assert_equals_msg(
        Some("Should have no snapshots !"),
        &0,
        &psdp.get_snapshot_count(),
    );
    drop(psdp);
    drop(dp);
    drop(writer);
    dir.close()
}

/// Port of `TestPersistentSnapshotDeletionPolicy.testSnapshotReleaseByGeneration`.
fn test_snapshot_release_by_generation() -> Result<()> {
    let mut f = SnapshotFixture::default();
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        f.get_config(
            &*random(),
            Some(get_deletion_policy(dir.as_directory())? as _),
        )?,
    )?;
    let dp = writer_psdp(&writer);
    let psdp = psdp_of(&*dp);
    f.prepare_index_and_snapshots(psdp, &writer, 1)?;
    writer.close()?;

    psdp.release_gen(f.snapshots[0].get_generation())?;

    let psdp = PersistentSnapshotDeletionPolicy::with_open_mode(
        Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
        dir.as_directory(),
        OpenMode::APPEND,
    )?;
    assert_equals_msg(
        Some("Should have no snapshots !"),
        &0,
        &psdp.get_snapshot_count(),
    );
    drop(psdp);
    drop(dp);
    drop(writer);
    dir.close()
}
