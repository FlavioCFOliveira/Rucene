//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDeletionPolicy.java`.
//!
//! Verify we can read the pre-2.1 file format, do searches against it, and
//! add documents to it.
//!
//! Java's policies are inner classes with plain `int` counters, read back
//! through `(KeepAllDeletionPolicy) writer.getConfig().getIndexDeletionPolicy()`;
//! Rucene shares a policy as `Arc<dyn IndexDeletionPolicy>` whose methods
//! take `&self`, so the counters are atomics and the test keeps the `Arc` it
//! installed, which is that same object.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::{IndexDeletionPolicy, KeepLastNCommitsDeletionPolicy};
use rucene::index::index_file_names::{file_name_from_generation, SEGMENTS};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::search::{Query, TermQuery};
use rucene::store::Directory;
use rucene::util::Version;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, verbose, IAE};
use crate::support::open_reader;

/// `TestDeletionPolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDeletionPolicy",
        vec![
            test(
                "testExpirationTimeDeletionPolicy",
                test_expiration_time_deletion_policy,
            )
            .annotate(Nightly),
            test("testKeepAllDeletionPolicy", test_keep_all_deletion_policy),
            test("testOpenPriorSnapshot", test_open_prior_snapshot),
            test(
                "testKeepNoneOnInitDeletionPolicy",
                test_keep_none_on_init_deletion_policy,
            ),
            test(
                "testKeepLastNDeletionPolicy",
                test_keep_last_n_deletion_policy,
            ),
            test(
                "testKeepLastNDeletionPolicyWithCreates",
                test_keep_last_n_deletion_policy_with_creates,
            ),
            test(
                "testKeepLastNCommitsDeletionPolicy",
                test_keep_last_n_commits_deletion_policy,
            ),
            test(
                "testKeepLastNCommitsDeletionPolicyWithZeroCommits",
                test_keep_last_n_commits_deletion_policy_with_zero_commits,
            ),
        ],
    )
}

/// `private void verifyCommitOrder(List<? extends IndexCommit> commits)`.
fn verify_commit_order(commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
    if commits.is_empty() {
        return Ok(());
    }
    let first_commit = &commits[0];
    let mut last =
        SegmentInfos::generation_from_segments_file_name(&first_commit.get_segments_file_name())?;
    assert_equals(&last, &first_commit.get_generation());
    for commit in &commits[1..] {
        let now =
            SegmentInfos::generation_from_segments_file_name(&commit.get_segments_file_name())?;
        Assert::assert_true_msg("SegmentInfos commits are out-of-order", now > last);
        assert_equals(&now, &commit.get_generation());
        last = now;
    }
    Ok(())
}

/// `class KeepAllDeletionPolicy extends IndexDeletionPolicy`.
struct KeepAllDeletionPolicy {
    num_on_init: AtomicI32,
    num_on_commit: AtomicI32,
    dir: Arc<dyn Directory>,
}

impl std::fmt::Debug for KeepAllDeletionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeepAllDeletionPolicy")
            .field("num_on_init", &self.num_on_init)
            .field("num_on_commit", &self.num_on_commit)
            .finish_non_exhaustive()
    }
}

impl KeepAllDeletionPolicy {
    fn new(dir: Arc<dyn Directory>) -> Self {
        KeepAllDeletionPolicy {
            num_on_init: AtomicI32::new(0),
            num_on_commit: AtomicI32::new(0),
            dir,
        }
    }
}

impl IndexDeletionPolicy for KeepAllDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDeletionPolicy$KeepAllDeletionPolicy"
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        verify_commit_order(commits)?;
        self.num_on_init.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        let last_commit = &commits[commits.len() - 1];
        let r = directory_reader::open(Arc::clone(&self.dir))?;
        let leaves = Arc::clone(&r).leaves()?.len() as i32;
        assert_equals_msg(
            Some(&format!(
                "lastCommit.segmentCount()={} vs IndexReader.segmentCount={}",
                last_commit.get_segment_count(),
                leaves
            )),
            &leaves,
            &last_commit.get_segment_count(),
        );
        r.close()?;
        verify_commit_order(commits)?;
        self.num_on_commit.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// This is useful for adding to a big index when you know readers are not
/// using it.
///
/// `class KeepNoneOnInitDeletionPolicy extends IndexDeletionPolicy`.
#[derive(Debug, Default)]
struct KeepNoneOnInitDeletionPolicy {
    num_on_init: AtomicI32,
    num_on_commit: AtomicI32,
}

impl IndexDeletionPolicy for KeepNoneOnInitDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDeletionPolicy$KeepNoneOnInitDeletionPolicy"
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        verify_commit_order(commits)?;
        self.num_on_init.fetch_add(1, Ordering::SeqCst);
        // On init, delete all commit points:
        for commit in commits {
            commit.delete()?;
            Assert::assert_true(commit.is_deleted());
        }
        Ok(())
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        verify_commit_order(commits)?;
        let size = commits.len();
        // Delete all but last one:
        for commit in &commits[..size - 1] {
            commit.delete()?;
        }
        self.num_on_commit.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `class KeepLastNDeletionPolicy extends IndexDeletionPolicy`.
#[derive(Debug)]
struct KeepLastNDeletionPolicy {
    num_on_init: AtomicI32,
    num_on_commit: AtomicI32,
    num_to_keep: usize,
    num_delete: AtomicI32,
    seen: Mutex<HashSet<String>>,
}

impl KeepLastNDeletionPolicy {
    fn new(num_to_keep: usize) -> Self {
        KeepLastNDeletionPolicy {
            num_on_init: AtomicI32::new(0),
            num_on_commit: AtomicI32::new(0),
            num_to_keep,
            num_delete: AtomicI32::new(0),
            seen: Mutex::new(HashSet::new()),
        }
    }

    fn do_deletes(&self, commits: &[Arc<dyn IndexCommit>], is_commit: bool) -> Result<()> {
        // Assert that we really are only called for each new
        // commit:
        if is_commit {
            let file_name = commits[commits.len() - 1].get_segments_file_name();
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if seen.contains(&file_name) {
                Throwable::new(
                    "java.lang.RuntimeException",
                    format!("onCommit was called twice on the same commit point: {file_name}"),
                )
                .throw()
            }
            seen.insert(file_name);
            self.num_on_commit.fetch_add(1, Ordering::SeqCst);
        }
        let size = commits.len();
        for commit in commits.iter().take(size.saturating_sub(self.num_to_keep)) {
            commit.delete()?;
            self.num_delete.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl IndexDeletionPolicy for KeepLastNDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDeletionPolicy$KeepLastNDeletionPolicy"
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        if verbose() {
            println!("TEST: onInit");
        }
        verify_commit_order(commits)?;
        self.num_on_init.fetch_add(1, Ordering::SeqCst);
        // do no deletions on init
        self.do_deletes(commits, false)
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        if verbose() {
            println!("TEST: onCommit");
        }
        verify_commit_order(commits)?;
        self.do_deletes(commits, true)
    }
}

/// `static long getCommitTime(IndexCommit commit)`.
fn get_commit_time(commit: &dyn IndexCommit) -> Result<i64> {
    let user_data = commit.get_user_data()?;
    let value = user_data
        .get("commitTime")
        .unwrap_or_else(|| crate::support::npe("getUserData().get(\"commitTime\")"));
    Ok(parse_long(value))
}

/// `Long.parseLong(s)`.
fn parse_long(s: &str) -> i64 {
    s.parse().unwrap_or_else(|_| {
        Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// Delete a commit only when it has been obsoleted by N seconds.
///
/// `class ExpirationTimeDeletionPolicy extends IndexDeletionPolicy`.
struct ExpirationTimeDeletionPolicy {
    #[allow(dead_code)]
    dir: Arc<dyn Directory>,
    expiration_time_seconds: f64,
    num_delete: AtomicI32,
}

impl std::fmt::Debug for ExpirationTimeDeletionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpirationTimeDeletionPolicy")
            .field("expiration_time_seconds", &self.expiration_time_seconds)
            .field("num_delete", &self.num_delete)
            .finish_non_exhaustive()
    }
}

impl IndexDeletionPolicy for ExpirationTimeDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestDeletionPolicy$ExpirationTimeDeletionPolicy"
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        if commits.is_empty() {
            return Ok(());
        }
        verify_commit_order(commits)?;
        self.on_commit(commits)
    }

    fn on_commit(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        verify_commit_order(commits)?;

        let last_commit = &commits[commits.len() - 1];

        // Any commit older than expireTime should be deleted:
        let expire_time =
            get_commit_time(&**last_commit)? as f64 / 1000.0 - self.expiration_time_seconds;

        for commit in commits {
            let mod_time = get_commit_time(&**commit)? as f64 / 1000.0;
            if !Arc::ptr_eq(commit, last_commit) && mod_time < expire_time {
                commit.delete()?;
                self.num_delete.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `IndexFileNames.fileNameFromGeneration(IndexFileNames.SEGMENTS, "", gen)`.
fn segments_file(gen: i64) -> String {
    file_name_from_generation(SEGMENTS, "", gen)
        .unwrap_or_else(|| crate::support::npe("fileNameFromGeneration"))
}

/// `writer.setLiveCommitData(commitData.entrySet())` with `commitTime =
/// String.valueOf(System.nanoTime())`.
fn set_commit_time(writer: &IndexWriter) -> Result<()> {
    writer.set_live_commit_data(vec![(
        "commitTime".to_string(),
        randomizedtesting::jdk::system::nano_time().to_string(),
    )])
}

/// Test "by time expiration" deletion policy:
///
/// Port of `TestDeletionPolicy.testExpirationTimeDeletionPolicy`.
// TODO: this wall-clock-dependent test doesn't seem to actually test any deletionpolicy logic?
fn test_expiration_time_deletion_policy() -> Result<()> {
    const SECONDS: f64 = 2.0;

    let mut dir = LuceneTestCase::new_directory();
    let policy = Arc::new(ExpirationTimeDeletionPolicy {
        dir: dir.as_directory(),
        expiration_time_seconds: SECONDS,
        num_delete: AtomicI32::new(0),
    });
    let conf = mock_iwc();
    conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
    conf.merge_policy().set_no_cfs_ratio(1.0)?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    set_commit_time(&writer)?;
    writer.commit()?;
    writer.close()?;

    let mut last_delete_time: i64 = 0;
    let target_num_delete = TestUtil::next_int(&*random(), 1, 5);
    while policy.num_delete.load(Ordering::SeqCst) < target_num_delete {
        // Record last time when writer performed deletes of
        // past commits
        last_delete_time = randomizedtesting::jdk::system::nano_time();
        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::APPEND)?;
        conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
        conf.merge_policy().set_no_cfs_ratio(1.0)?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        for _ in 0..17 {
            add_doc(&writer)?;
        }
        set_commit_time(&writer)?;
        writer.commit()?;
        writer.close()?;

        std::thread::sleep(std::time::Duration::from_millis(
            (1000.0 * (SECONDS / 5.0)) as u64,
        ));
    }

    // Then simplistic check: just verify that the
    // segments_N's that still exist are in fact within SECONDS
    // seconds of the last one's mod time, and, that I can
    // open a reader on each:
    let mut gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;

    let mut _file_name = segments_file(gen);
    let mut one_second_resolution = true;

    while gen > 0 {
        let attempt = (|| -> Result<()> {
            let reader = open_reader(&dir)?;
            reader.close()?;
            _file_name = segments_file(gen);

            // if we are on a filesystem that seems to have only
            // 1 second resolution, allow +1 second in commit
            // age tolerance:
            let sis = SegmentInfos::read_commit(dir.as_directory(), &_file_name)?;
            assert_equals(&Version::LATEST, &sis.commit_lucene_version());
            assert_equals(&Some(Version::LATEST), &sis.min_segment_lucene_version());
            let mod_time = parse_long(
                sis.user_data()
                    .get("commitTime")
                    .unwrap_or_else(|| crate::support::npe("getUserData().get(\"commitTime\")")),
            );
            one_second_resolution &= (mod_time % 1000) == 0;
            let leeway =
                ((SECONDS + if one_second_resolution { 1.0 } else { 0.0 }) * 1000.0) as i64;

            Assert::assert_true_msg(
                &format!(
                    "commit point was older than {SECONDS} seconds ({} ms) but did not get deleted ",
                    last_delete_time - mod_time
                ),
                last_delete_time - mod_time <= leeway,
            );
            Ok(())
        })();
        match attempt {
            Ok(()) => {}
            // OK
            Err(e) if is_instance(&e, "java.io.IOException") => break,
            Err(e) => return Err(e),
        }

        dir.as_directory().delete_file(&segments_file(gen))?;
        gen -= 1;
    }

    dir.close()
}

/// Test a silly deletion policy that keeps all commits around.
///
/// Port of `TestDeletionPolicy.testKeepAllDeletionPolicy`.
fn test_keep_all_deletion_policy() -> Result<()> {
    for pass in 0..2 {
        if verbose() {
            println!("TEST: cycle pass={pass}");
        }

        let use_compound_file = (pass % 2) != 0;

        let mut dir = LuceneTestCase::new_directory();

        let policy = Arc::new(KeepAllDeletionPolicy::new(dir.as_directory()));
        let conf = mock_iwc();
        conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
        conf.set_max_buffered_docs(10)?;
        conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        conf.merge_policy()
            .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        for _ in 0..107 {
            add_doc(&writer)?;
        }
        writer.close()?;

        let needs_merging = {
            let r = open_reader(&dir)?;
            let needs_merging = Arc::clone(&r).leaves()?.len() != 1;
            r.close()?;
            needs_merging
        };
        if needs_merging {
            let conf = mock_iwc();
            conf.set_open_mode(OpenMode::APPEND)?;
            conf.set_index_deletion_policy(Some(
                Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
            ))?;
            conf.merge_policy()
                .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
            if verbose() {
                println!("TEST: open writer for forceMerge");
            }
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            writer.force_merge(1)?;
            writer.close()?;
        }

        assert_equals(
            &(if needs_merging { 2 } else { 1 }),
            &policy.num_on_init.load(Ordering::SeqCst),
        );

        // If we are not auto committing then there should
        // be exactly 2 commits (one per close above):
        assert_equals(
            &(1 + if needs_merging { 1 } else { 0 }),
            &policy.num_on_commit.load(Ordering::SeqCst),
        );

        // Test listCommits
        let commits = directory_reader::list_commits(dir.as_directory())?;
        // 2 from closing writer
        assert_equals(&(1 + if needs_merging { 1 } else { 0 }), &commits.len());

        // Make sure we can open a reader on each commit:
        for commit in &commits {
            let r = directory_reader::open_with_commit(&**commit)?;
            r.close()?;
        }

        // Simplistic check: just verify all segments_N's still
        // exist, and, I can open a reader on each:
        let mut gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;
        while gen > 0 {
            let reader = open_reader(&dir)?;
            reader.close()?;
            dir.as_directory().delete_file(&segments_file(gen))?;
            gen -= 1;

            if gen > 0 {
                // Now that we've removed a commit point, which
                // should have orphan'd at least one index file.
                // Open & close a writer and assert that it
                // actually removed something:
                let pre_count = dir.as_directory().list_all()?.len();
                let conf = mock_iwc();
                conf.set_open_mode(OpenMode::APPEND)?;
                conf.set_index_deletion_policy(Some(
                    Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
                ))?;
                let writer = IndexWriter::new(dir.as_directory(), conf)?;
                writer.close()?;
                let post_count = dir.as_directory().list_all()?.len();
                Assert::assert_true(post_count < pre_count);
            }
        }

        drop(policy);
        dir.close()?;
    }
    Ok(())
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setIndexDeletionPolicy(policy)
/// .setIndexCommit(lastCommit).setMergePolicy(newLogMergePolicy(10))`.
fn prior_commit_iwc(
    policy: Option<&Arc<KeepAllDeletionPolicy>>,
    last_commit: &Arc<dyn IndexCommit>,
) -> Result<IndexWriterConfig> {
    let conf = mock_iwc();
    if let Some(policy) = policy {
        conf.set_index_deletion_policy(Some(Arc::clone(policy) as Arc<dyn IndexDeletionPolicy>))?;
    }
    conf.set_index_commit(Some(Arc::clone(last_commit)));
    conf.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    Ok(conf)
}

/// Uses KeepAllDeletionPolicy to keep all commits around, then, opens a new
/// IndexWriter on a previous commit point.
///
/// Port of `TestDeletionPolicy.testOpenPriorSnapshot`.
fn test_open_prior_snapshot() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let policy = Arc::new(KeepAllDeletionPolicy::new(dir.as_directory()));
    let conf = mock_iwc();
    conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
    conf.set_max_buffered_docs(2)?;
    conf.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    for i in 0..10 {
        add_doc(&writer)?;
        if (1 + i) % 2 == 0 {
            writer.commit()?;
        }
    }
    writer.close()?;

    let commits = directory_reader::list_commits(dir.as_directory())?;
    assert_equals(&5, &commits.len());
    let mut last_commit: Option<Arc<dyn IndexCommit>> = None;
    for commit in &commits {
        if last_commit
            .as_ref()
            .is_none_or(|l| commit.get_generation() > l.get_generation())
        {
            last_commit = Some(Arc::clone(commit));
        }
    }
    Assert::assert_true(last_commit.is_some());
    let last_commit = last_commit.expect("checked by assertTrue");

    // Now add 1 doc and merge
    let conf = mock_iwc();
    conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    add_doc(&writer)?;
    assert_equals(&11, &writer.get_doc_stats()?.num_docs);
    writer.force_merge(1)?;
    writer.close()?;

    assert_equals(
        &6,
        &directory_reader::list_commits(dir.as_directory())?.len(),
    );

    // Now open writer on the commit just before merge:
    let writer = IndexWriter::new(
        dir.as_directory(),
        prior_commit_iwc(Some(&policy), &last_commit)?,
    )?;
    assert_equals(&10, &writer.get_doc_stats()?.num_docs);

    // Should undo our rollback:
    writer.rollback()?;

    let r = open_reader(&dir)?;
    // Still merged, still 11 docs
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    assert_equals(&11, &r.num_docs());
    r.close()?;

    let writer = IndexWriter::new(
        dir.as_directory(),
        prior_commit_iwc(Some(&policy), &last_commit)?,
    )?;
    assert_equals(&10, &writer.get_doc_stats()?.num_docs);
    // Commits the rollback:
    writer.close()?;

    // Now 7 because we made another commit
    assert_equals(
        &7,
        &directory_reader::list_commits(dir.as_directory())?.len(),
    );

    let r = open_reader(&dir)?;
    // Not fully merged because we rolled it back, and now only
    // 10 docs
    Assert::assert_true(Arc::clone(&r).leaves()?.len() > 1);
    assert_equals(&10, &r.num_docs());
    r.close()?;

    // Re-merge
    let conf = mock_iwc();
    conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    writer.force_merge(1)?;
    writer.close()?;

    let r = open_reader(&dir)?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    assert_equals(&10, &r.num_docs());
    r.close()?;

    // Now open writer on the commit just before merging,
    // but this time keeping only the last commit:
    let writer = IndexWriter::new(dir.as_directory(), prior_commit_iwc(None, &last_commit)?)?;
    assert_equals(&10, &writer.get_doc_stats()?.num_docs);

    // Reader still sees fully merged index, because writer
    // opened on the prior commit has not yet committed:
    let r = open_reader(&dir)?;
    assert_equals(&1, &Arc::clone(&r).leaves()?.len());
    assert_equals(&10, &r.num_docs());
    r.close()?;

    writer.close()?;

    // Now reader sees not-fully-merged index:
    let r = open_reader(&dir)?;
    Assert::assert_true(Arc::clone(&r).leaves()?.len() > 1);
    assert_equals(&10, &r.num_docs());
    r.close()?;

    drop((commits, last_commit, policy));
    dir.close()
}

/// Test keeping NO commit points. This is a viable and useful case eg where
/// you want to build a big index and you know there are no readers.
///
/// Port of `TestDeletionPolicy.testKeepNoneOnInitDeletionPolicy`.
fn test_keep_none_on_init_deletion_policy() -> Result<()> {
    for pass in 0..2 {
        let use_compound_file = (pass % 2) != 0;

        let mut dir = LuceneTestCase::new_directory();

        let policy = Arc::new(KeepNoneOnInitDeletionPolicy::default());
        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::CREATE)?;
        conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
        conf.set_max_buffered_docs(10)?;
        conf.merge_policy()
            .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        for _ in 0..107 {
            add_doc(&writer)?;
        }
        writer.close()?;

        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::APPEND)?;
        conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
        conf.merge_policy().set_no_cfs_ratio(1.0)?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        writer.force_merge(1)?;
        writer.close()?;

        assert_equals(&2, &policy.num_on_init.load(Ordering::SeqCst));
        // If we are not auto committing then there should
        // be exactly 2 commits (one per close above):
        assert_equals(&2, &policy.num_on_commit.load(Ordering::SeqCst));

        // Simplistic check: just verify the index is in fact
        // readable:
        let reader = open_reader(&dir)?;
        reader.close()?;

        dir.close()?;
    }
    Ok(())
}

/// `try { DirectoryReader.open(dir) … } catch (IOException e) { if (i != N)
/// throw e; }`: the `IOException` the open throws, when it throws one.
fn io_exception_of(outcome: Result<()>) -> Result<Option<LuceneError>> {
    match outcome {
        Ok(()) => Ok(None),
        Err(e) if is_instance(&e, "java.io.IOException") => Ok(Some(e)),
        Err(e) => Err(e),
    }
}

/// Test a deletion policy that keeps last N commits.
///
/// Port of `TestDeletionPolicy.testKeepLastNDeletionPolicy`.
fn test_keep_last_n_deletion_policy() -> Result<()> {
    const N: usize = 5;

    for pass in 0..2 {
        let use_compound_file = (pass % 2) != 0;

        let mut dir = LuceneTestCase::new_directory();

        let policy = Arc::new(KeepLastNDeletionPolicy::new(N));
        for _ in 0..N + 1 {
            let conf = mock_iwc();
            conf.set_open_mode(OpenMode::CREATE)?;
            conf.set_index_deletion_policy(Some(
                Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
            ))?;
            conf.set_max_buffered_docs(10)?;
            conf.merge_policy()
                .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            for _ in 0..17 {
                add_doc(&writer)?;
            }
            writer.force_merge(1)?;
            writer.close()?;
        }

        Assert::assert_true(policy.num_delete.load(Ordering::SeqCst) > 0);
        assert_equals(
            &((N + 1) as i32),
            &policy.num_on_init.load(Ordering::SeqCst),
        );
        assert_equals(
            &((N + 1) as i32),
            &policy.num_on_commit.load(Ordering::SeqCst),
        );

        // Simplistic check: just verify only the past N segments_N's still
        // exist, and, I can open a reader on each:
        let mut gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;
        for i in 0..N + 1 {
            let caught = io_exception_of((|| {
                let reader = open_reader(&dir)?;
                reader.close()?;
                if i == N {
                    Assert::fail(&format!("should have failed on commits prior to last {N}"));
                }
                Ok(())
            })())?;
            if let Some(e) = caught {
                if i != N {
                    return Err(e);
                }
            }
            if i < N {
                dir.as_directory().delete_file(&segments_file(gen))?;
            }
            gen -= 1;
        }

        dir.close()?;
    }
    Ok(())
}

/// `private void addDocWithID(IndexWriter writer, int id)`.
fn add_doc_with_id(writer: &IndexWriter, id: usize) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        &id.to_string(),
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `private void addDoc(IndexWriter writer)`.
fn add_doc(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `newSearcher(reader).search(query, 1000).scoreDocs.length`.
fn hit_count(
    dir: &TestDirectory,
    query: &Arc<dyn Query>,
) -> Result<(
    usize,
    Arc<dyn rucene::index::directory_reader::DirectoryReader>,
)> {
    let reader = open_reader(dir)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let hits = searcher.search(Arc::clone(query), 1000)?.score_docs;
    Ok((hits.len(), reader))
}

/// Test a deletion policy that keeps last N commits around, through
/// creates.
///
/// Port of `TestDeletionPolicy.testKeepLastNDeletionPolicyWithCreates`.
fn test_keep_last_n_deletion_policy_with_creates() -> Result<()> {
    const N: usize = 10;

    for pass in 0..2 {
        let use_compound_file = (pass % 2) != 0;

        let mut dir = LuceneTestCase::new_directory();
        let policy = Arc::new(KeepLastNDeletionPolicy::new(N));
        let conf = mock_iwc();
        conf.set_open_mode(OpenMode::CREATE)?;
        conf.set_index_deletion_policy(Some(Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>))?;
        conf.set_max_buffered_docs(10)?;
        conf.merge_policy()
            .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        writer.close()?;
        let search_term = Term::from_text("content", "aaa");
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(search_term));

        for i in 0..N + 1 {
            let conf = mock_iwc();
            conf.set_open_mode(OpenMode::APPEND)?;
            conf.set_index_deletion_policy(Some(
                Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
            ))?;
            conf.set_max_buffered_docs(10)?;
            conf.merge_policy()
                .set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            for j in 0..17 {
                add_doc_with_id(&writer, i * (N + 1) + j)?;
            }
            // this is a commit
            writer.close()?;
            let conf =
                IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
            conf.set_index_deletion_policy(Some(
                Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
            ))?;
            conf.set_merge_policy(Some(NoMergePolicy::instance()))?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            writer.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &(i * (N + 1) + 3).to_string(),
            )])?;
            // this is a commit
            writer.close()?;
            let (hits, reader) = hit_count(&dir, &query)?;
            assert_equals(&16, &hits);
            reader.close()?;

            let conf = mock_iwc();
            conf.set_open_mode(OpenMode::CREATE)?;
            conf.set_index_deletion_policy(Some(
                Arc::clone(&policy) as Arc<dyn IndexDeletionPolicy>
            ))?;
            let writer = IndexWriter::new(dir.as_directory(), conf)?;
            // This will not commit: there are no changes
            // pending because we opened for "create":
            writer.close()?;
        }

        assert_equals(
            &(3 * (N + 1) as i32 + 1),
            &policy.num_on_init.load(Ordering::SeqCst),
        );
        assert_equals(
            &(3 * (N + 1) as i32 + 1),
            &policy.num_on_commit.load(Ordering::SeqCst),
        );

        let (hits, rw_reader) = hit_count(&dir, &query)?;
        assert_equals(&0, &hits);

        // Simplistic check: just verify only the past N segments_N's still
        // exist, and, I can open a reader on each:
        let mut gen = SegmentInfos::get_last_commit_generation_dir(&*dir.as_directory())?;

        let mut expected_count = 0;

        rw_reader.close()?;

        for i in 0..N + 1 {
            let caught = io_exception_of((|| {
                let (hits, reader) = hit_count(&dir, &query)?;

                // Work backwards in commits on what the expected
                // count should be.
                assert_equals(&expected_count, &hits);
                if expected_count == 0 {
                    expected_count = 16;
                } else if expected_count == 16 {
                    expected_count = 17;
                } else if expected_count == 17 {
                    expected_count = 0;
                }
                reader.close()?;
                if i == N {
                    Assert::fail(&format!("should have failed on commits before last {N}"));
                }
                Ok(())
            })())?;
            if let Some(e) = caught {
                if i != N {
                    return Err(e);
                }
            }
            if i < N {
                dir.as_directory().delete_file(&segments_file(gen))?;
            }
            gen -= 1;
        }

        dir.close()?;
    }
    Ok(())
}

/// Port of `TestDeletionPolicy.testKeepLastNCommitsDeletionPolicy`.
fn test_keep_last_n_commits_deletion_policy() -> Result<()> {
    let num_commits_to_keep = 3;
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    conf.set_index_deletion_policy(Some(Arc::new(KeepLastNCommitsDeletionPolicy::new(
        num_commits_to_keep,
    )?)))?;

    assert_equals(
        "org.apache.lucene.index.KeepLastNCommitsDeletionPolicy",
        conf.index_deletion_policy().java_class_name(),
    );

    // Create an index and make several commits
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    for _ in 0..5 {
        add_doc(&writer)?;
        writer.commit()?;
    }

    writer.close()?;

    // Check that only the last N commits are kept
    let commits = directory_reader::list_commits(dir.as_directory())?;
    assert_equals(&(num_commits_to_keep as usize), &commits.len());

    // Verify that we can open and read from each of the remaining commits
    for commit in &commits {
        let reader = directory_reader::open_with_commit(&**commit)?;
        Assert::assert_true(reader.num_docs() > 0);
        reader.close()?;
    }

    // Check that the retained commits are the most recent ones
    let latest_gen = commits[commits.len() - 1].get_generation();
    for i in 0..num_commits_to_keep as usize {
        assert_equals(
            &(latest_gen - i as i64),
            &commits[commits.len() - 1 - i].get_generation(),
        );
    }

    drop(commits);
    dir.close()
}

/// Port of `TestDeletionPolicy.testKeepLastNCommitsDeletionPolicyWithZeroCommits`.
fn test_keep_last_n_commits_deletion_policy_with_zero_commits() -> Result<()> {
    let num_commits_to_keep = 0;
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let expected = expect_throws(IAE, || -> Result<()> {
        conf.set_index_deletion_policy(Some(Arc::new(KeepLastNCommitsDeletionPolicy::new(
            num_commits_to_keep,
        )?)))?;
        Ok(())
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_default()
            .contains("number of recent commits to keep must be positive"),
    );
    Ok(())
}
