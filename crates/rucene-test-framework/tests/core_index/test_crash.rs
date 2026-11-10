//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCrash.java`.
//!
//! Java recovers the `MockDirectoryWrapper` with
//! `(MockDirectoryWrapper) writer.getDirectory()`; the port keeps the
//! directory it created beside the writer (`initIndex` returns both), which
//! is that same object.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::store::NoLockFactory;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, verbose};
use crate::support::{as_cms, open_reader};

/// `TestCrash`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCrash",
        vec![
            test("testCrashWhileIndexing", test_crash_while_indexing),
            test("testWriterAfterCrash", test_writer_after_crash),
            test("testCrashAfterReopen", test_crash_after_reopen),
            test("testCrashAfterClose", test_crash_after_close),
            test("testCrashAfterCloseNoWait", test_crash_after_close_no_wait),
        ],
    )
}

/// `newMockDirectory(random, NoLockFactory.INSTANCE)`.
fn new_no_lock_mock_directory(random: &dyn Random) -> TestDirectory {
    TestDirectory::Mock(LuceneTestCase::new_mock_directory_lock_factory(
        random,
        Arc::new(*NoLockFactory::instance()),
    ))
}

/// The directory's `MockDirectoryWrapper`.
fn mock(dir: &TestDirectory) -> &MockDirectoryWrapper {
    dir.as_mock()
        .expect("INVARIANT: the directory is a MockDirectoryWrapper")
}

/// `private IndexWriter initIndex(Random random, boolean initialCommit)`.
fn init_index_new(
    random: &dyn Random,
    initial_commit: bool,
) -> Result<(Arc<IndexWriter>, TestDirectory)> {
    let dir = new_no_lock_mock_directory(random);
    let writer = init_index(random, &dir, initial_commit, true)?;
    Ok((writer, dir))
}

/// `private IndexWriter initIndex(Random random, MockDirectoryWrapper dir,
/// boolean initialCommit, boolean commitOnClose)`.
fn init_index(
    random: &dyn Random,
    dir: &TestDirectory,
    initial_commit: bool,
    commit_on_close: bool,
) -> Result<Arc<IndexWriter>> {
    let iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(random))));
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_commit_on_close(commit_on_close);
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let ms = writer.get_config().merge_scheduler();
    as_cms(&ms).set_suppress_exceptions();
    if initial_commit {
        writer.commit()?;
    }

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "id",
        "0",
        Store::NO,
    )));
    for _ in 0..157 {
        writer.add_document(&doc)?;
    }

    Ok(writer)
}

/// `private void crash(final IndexWriter writer)`.
fn crash(writer: &IndexWriter, dir: &TestDirectory) -> Result<()> {
    let ms = writer.get_config().merge_scheduler();
    let cms = as_cms(&ms);
    cms.sync();
    mock(dir).crash()?;
    cms.sync();
    mock(dir).clear_crash();
    Ok(())
}

/// `Directory dir2 = newDirectory(dir); dir.close(); new
/// RandomIndexWriter(random(), dir2).close(); dir2.close();`
fn copy_and_reopen(mut dir: TestDirectory) -> Result<()> {
    // Make a new dir, copying from the crashed dir, and
    // open IW on it, to confirm IW "recovers" after a
    // crash:
    let mut dir2 = LuceneTestCase::new_directory_copy(&*dir.as_directory())?;
    dir.close()?;

    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir2.as_directory())?.close()?;
    dir2.close()
}

/// Port of `TestCrash.testCrashWhileIndexing`.
fn test_crash_while_indexing() -> Result<()> {
    // This test relies on being able to open a reader before any commit
    // happened, so we must create an initial commit just to allow that, but
    // before any documents were added.
    let (writer, dir) = init_index_new(&*random(), true)?;

    // We create leftover files because merging could be
    // running when we crash:
    mock(&dir).set_assert_no_unrefenced_files_on_close(false);

    crash(&writer, &dir)?;

    let reader = open_reader(&dir)?;
    Assert::assert_true(reader.num_docs() < 157);
    reader.close()?;

    copy_and_reopen(dir)
}

/// Port of `TestCrash.testWriterAfterCrash`.
fn test_writer_after_crash() -> Result<()> {
    // This test relies on being able to open a reader before any commit
    // happened, so we must create an initial commit just to allow that, but
    // before any documents were added.
    if verbose() {
        println!("TEST: initIndex");
    }
    let (writer, dir) = init_index_new(&*random(), true)?;
    if verbose() {
        println!("TEST: done initIndex");
    }

    // We create leftover files because merging could be
    // running / store files could be open when we crash:
    mock(&dir).set_assert_no_unrefenced_files_on_close(false);

    if verbose() {
        println!("TEST: now crash");
    }
    crash(&writer, &dir)?;
    let writer = init_index(&*random(), &dir, false, true)?;
    writer.close()?;

    let reader = open_reader(&dir)?;
    Assert::assert_true(reader.num_docs() < 314);
    reader.close()?;

    copy_and_reopen(dir)
}

/// Port of `TestCrash.testCrashAfterReopen`.
fn test_crash_after_reopen() -> Result<()> {
    let (writer, dir) = init_index_new(&*random(), false)?;

    // We create leftover files because merging could be
    // running when we crash:
    mock(&dir).set_assert_no_unrefenced_files_on_close(false);

    writer.close()?;
    let writer = init_index(&*random(), &dir, false, true)?;
    assert_equals(&314, &writer.get_doc_stats()?.max_doc);
    crash(&writer, &dir)?;

    let reader = open_reader(&dir)?;
    Assert::assert_true(reader.num_docs() >= 157);
    reader.close()?;

    copy_and_reopen(dir)
}

/// Port of `TestCrash.testCrashAfterClose`.
fn test_crash_after_close() -> Result<()> {
    let (writer, mut dir) = init_index_new(&*random(), false)?;

    writer.close()?;
    mock(&dir).crash()?;

    let reader = open_reader(&dir)?;
    assert_equals(&157, &reader.num_docs());
    reader.close()?;
    dir.close()
}

/// Port of `TestCrash.testCrashAfterCloseNoWait`.
fn test_crash_after_close_no_wait() -> Result<()> {
    let random = random();
    let mut dir = new_no_lock_mock_directory(&*random);
    let writer = init_index(&*random, &dir, false, false)?;

    let committed = writer.commit();
    // `finally`
    writer.close()?;
    committed?;

    mock(&dir).crash()?;

    let reader = open_reader(&dir)?;
    assert_equals(&157, &reader.num_docs());
    reader.close()?;
    dir.close()
}
