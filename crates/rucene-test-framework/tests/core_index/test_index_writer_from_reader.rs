//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterFromReader.java`.
//!
//! * `r.getIndexCommit()` handed to `IndexWriterConfig.setIndexCommit` is the
//!   reader's `index_commit()` shared as an `Arc`.
//! * `IOUtils.close(a, b, …)` closes each resource in order through
//!   `IOUtils::close`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::IndexDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::terms::Term;
use rucene::util::IOUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class, random, test, verbose};
use crate::support::npe;

const IAE: &str = "java.lang.IllegalArgumentException";

/// `TestIndexWriterFromReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterFromReader",
        vec![
            test("testRightAfterCommit", test_right_after_commit),
            test("testFromNonNRTReader", test_from_non_nrt_reader),
            test("testWithNoFirstCommit", test_with_no_first_commit),
            test("testAfterCommitThenIndex", test_after_commit_then_index),
            test("testNRTRollback", test_nrt_rollback),
            test("testRandom", test_random),
            test("testConsistentFieldNumbers", test_consistent_field_numbers),
            test("testInvalidOpenMode", test_invalid_open_mode),
            test("testOnClosedReader", test_on_closed_reader),
            test("testStaleNRTReader", test_stale_nrt_reader),
            test("testAfterRollback", test_after_rollback),
            test(
                "testAfterCommitThenIndexKeepCommits",
                test_after_commit_then_index_keep_commits,
            ),
        ],
    )
}

/// `r.getIndexCommit()`.
fn index_commit(r: &Arc<dyn DirectoryReader>) -> Result<Arc<dyn IndexCommit>> {
    Ok(Arc::from(r.index_commit()?))
}

/// `newIndexWriterConfig()` with `setIndexCommit(r.getIndexCommit())`.
fn iwc_from(r: &Arc<dyn DirectoryReader>) -> Result<IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_commit(Some(index_commit(r)?));
    Ok(iwc)
}

/// `new IndexWriter(dir, newIndexWriterConfig())`.
fn new_writer(dir: &Arc<dyn rucene::store::Directory>) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(Arc::clone(dir), LuceneTestCase::new_index_writer_config())
}

/// Pull NRT reader immediately after writer has committed
/// Port of `TestIndexWriterFromReader.testRightAfterCommit`.
fn test_right_after_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());
    w.close()?;

    let iwc = iwc_from(&r)?;
    let w2 = IndexWriter::new(Arc::clone(&d), iwc)?;
    r.close()?;

    assert_equals(&1, &w2.get_doc_stats()?.max_doc);
    w2.add_document(&Document::new())?;
    assert_equals(&2, &w2.get_doc_stats()?.max_doc);
    w2.close()?;

    let r2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &r2.max_doc());
    r2.close()?;
    drop(d);
    dir.close()
}

/// Open from non-NRT reader
/// Port of `TestIndexWriterFromReader.testFromNonNRTReader`.
fn test_from_non_nrt_reader() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.close()?;

    let r = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&1, &r.max_doc());
    let iwc = iwc_from(&r)?;
    let w2 = IndexWriter::new(Arc::clone(&d), iwc)?;
    assert_equals(&1, &r.max_doc());
    r.close()?;

    assert_equals(&1, &w2.get_doc_stats()?.max_doc);
    w2.add_document(&Document::new())?;
    assert_equals(&2, &w2.get_doc_stats()?.max_doc);
    w2.close()?;

    let r2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &r2.max_doc());
    r2.close()?;
    drop(d);
    dir.close()
}

/// Pull NRT reader from a writer on a new index with no commit:
/// Port of `TestIndexWriterFromReader.testWithNoFirstCommit`.
fn test_with_no_first_commit() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.rollback()?;

    let iwc = iwc_from(&r)?;
    let expected = expect_throws(IAE, || IndexWriter::new(Arc::clone(&d), iwc));
    assert_equals(
        &Some("cannot use IndexWriterConfig.setIndexCommit() when index has no commit"),
        &expected.message(),
    );

    r.close()?;
    drop(d);
    dir.close()
}

/// Pull NRT reader after writer has committed and then indexed another doc:
/// Port of `TestIndexWriterFromReader.testAfterCommitThenIndex`.
fn test_after_commit_then_index() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;
    w.add_document(&Document::new())?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &r.max_doc());
    w.close()?;

    let iwc = iwc_from(&r)?;
    let expected = expect_throws(IAE, || IndexWriter::new(Arc::clone(&d), iwc));
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_else(|| npe("message"))
            .contains("the provided reader is stale: its prior commit file"),
    );

    r.close()?;
    drop(d);
    dir.close()
}

/// NRT rollback: pull NRT reader after writer has committed and then before indexing another doc
/// Port of `TestIndexWriterFromReader.testNRTRollback`.
fn test_nrt_rollback() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());

    // Add another doc
    w.add_document(&Document::new())?;
    assert_equals(&2, &w.get_doc_stats()?.max_doc);

    w.close()?;

    let iwc = iwc_from(&r)?;
    let expected = expect_throws(IAE, || IndexWriter::new(Arc::clone(&d), iwc));
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_else(|| npe("message"))
            .contains("the provided reader is stale: its prior commit file"),
    );

    r.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterFromReader.testRandom`.
fn test_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let num_ops = LuceneTestCase::at_least(100);

    let mut w = new_writer(&d)?;

    // We must have a starting commit for this test because whenever we rollback with
    // an NRT reader, the commit before that NRT reader must exist
    w.commit()?;

    let mut r = directory_reader::open_with_writer(&*w)?;
    let mut nrt_reader_num_docs = 0;
    let mut writer_num_docs = 0;

    let mut commit_after_nrt = false;

    let mut live_ids: HashSet<i32> = HashSet::new();
    let mut nrt_live_ids: HashSet<i32> = HashSet::new();

    for op in 0..num_ops {
        if verbose() {
            println!(
                "\nITER op={op} nrtReaderNumDocs={nrt_reader_num_docs} writerNumDocs={writer_num_docs} r={r} r.numDocs()={}",
                r.num_docs()
            );
        }

        assert_equals(&nrt_reader_num_docs, &r.num_docs());
        let x = random().next_int_bound(5);

        match x {
            0 => {
                if verbose() {
                    println!("  add doc id={op}");
                }
                // add doc
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_string_field(
                    "id",
                    &format!("{op}"),
                    Store::NO,
                )));
                w.add_document(&doc)?;
                live_ids.insert(op);
                writer_num_docs += 1;
            }
            1 => {
                if verbose() {
                    println!("  delete doc");
                }
                // delete docs
                if !live_ids.is_empty() {
                    let id = random().next_int_bound(op);
                    if verbose() {
                        println!("    id={id}");
                    }
                    w.delete_documents_by_terms(vec![Term::from_text("id", &format!("{id}"))])?;
                    if live_ids.remove(&id) {
                        if verbose() {
                            println!("    really deleted");
                        }
                        writer_num_docs -= 1;
                    }
                } else if verbose() {
                    println!("    nothing to delete yet");
                }
            }
            2 => {
                // reopen NRT reader
                if verbose() {
                    println!("  reopen NRT reader");
                }
                match directory_reader::open_if_changed(Arc::clone(&r))? {
                    Some(r2) => {
                        r.close()?;
                        r = r2;
                        if verbose() {
                            println!(
                                "    got new reader oldNumDocs={nrt_reader_num_docs} newNumDocs={writer_num_docs}"
                            );
                        }
                        nrt_reader_num_docs = writer_num_docs;
                        nrt_live_ids = live_ids.clone();
                    }
                    None => {
                        if verbose() {
                            println!("    reader is unchanged");
                        }
                        assert_equals(&nrt_reader_num_docs, &r.num_docs());
                    }
                }
                commit_after_nrt = false;
            }
            3 => {
                if !commit_after_nrt {
                    // rollback writer to last nrt reader
                    if random().next_boolean() {
                        if verbose() {
                            println!(
                                "  close writer and open new writer from non-NRT reader numDocs={}",
                                w.get_doc_stats()?.num_docs
                            );
                        }
                        w.close()?;
                        r.close()?;
                        r = directory_reader::open(Arc::clone(&d))?;
                        assert_equals(&writer_num_docs, &r.num_docs());
                        nrt_reader_num_docs = writer_num_docs;
                        nrt_live_ids = live_ids.clone();
                    } else {
                        if verbose() {
                            println!(
                                "  rollback writer and open new writer from NRT reader numDocs={}",
                                w.get_doc_stats()?.num_docs
                            );
                        }
                        w.rollback()?;
                    }
                    let iwc = iwc_from(&r)?;
                    w = IndexWriter::new(Arc::clone(&d), iwc)?;
                    writer_num_docs = nrt_reader_num_docs;
                    live_ids = nrt_live_ids.clone();
                    r.close()?;
                    r = directory_reader::open_with_writer(&*w)?;
                }
            }
            4 => {
                if verbose() {
                    println!("    commit");
                }
                w.commit()?;
                commit_after_nrt = true;
            }
            _ => {}
        }
    }

    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexWriterFromReader.testConsistentFieldNumbers`.
fn test_consistent_field_numbers() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    // Empty first commit:
    w.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f0",
        "foo",
        Store::NO,
    )));
    w.add_document(&doc)?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f1",
        "foo",
        Store::NO,
    )));
    w.add_document(&doc)?;

    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    r.close()?;
    assert_equals(&2, &r2.max_doc());
    w.rollback()?;

    let iwc = iwc_from(&r2)?;
    let w2 = IndexWriter::new(Arc::clone(&d), iwc)?;
    r2.close()?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f1",
        "foo",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "f0",
        "foo",
        Store::NO,
    )));
    w2.add_document(&doc)?;
    w2.close()?;
    drop(d);
    dir.close()
}

/// Port of `TestIndexWriterFromReader.testInvalidOpenMode`.
fn test_invalid_open_mode() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());
    w.close()?;

    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_index_commit(Some(index_commit(&r)?));
    let expected = expect_throws(IAE, || IndexWriter::new(Arc::clone(&d), iwc));
    assert_equals(
        &Some("cannot use IndexWriterConfig.setIndexCommit() with OpenMode.CREATE"),
        &expected.message(),
    );

    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexWriterFromReader.testOnClosedReader`.
fn test_on_closed_reader() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());
    let commit = index_commit(&r)?;
    r.close()?;
    w.close()?;

    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_index_commit(Some(commit));
    expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
        IndexWriter::new(Arc::clone(&d), iwc)
    });

    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexWriterFromReader.testStaleNRTReader`.
fn test_stale_nrt_reader() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r.max_doc());
    w.add_document(&Document::new())?;

    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);
    r2.unwrap_or_else(|| npe("r2")).close()?;
    w.rollback()?;

    let iwc = iwc_from(&r)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    assert_equals(&1, &w.get_doc_stats()?.num_docs);

    r.close()?;
    let r3 = directory_reader::open_with_writer(&*w)?;
    assert_equals(&1, &r3.num_docs());

    w.add_document(&Document::new())?;
    let r4 = directory_reader::open_if_changed(Arc::clone(&r3))?.unwrap_or_else(|| npe("r4"));
    r3.close()?;

    assert_equals(&2, &r4.num_docs());
    r4.close()?;
    w.close()?;

    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexWriterFromReader.testAfterRollback`.
fn test_after_rollback() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let w = new_writer(&d)?;
    w.add_document(&Document::new())?;
    w.commit()?;
    w.add_document(&Document::new())?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &r.max_doc());
    w.rollback()?;

    let iwc = iwc_from(&r)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    assert_equals(&2, &w.get_doc_stats()?.num_docs);
    r.close()?;
    w.close()?;

    let r2 = directory_reader::open(Arc::clone(&d))?;
    assert_equals(&2, &r2.num_docs());
    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The anonymous `IndexDeletionPolicy` of
/// `testAfterCommitThenIndexKeepCommits`: keep all commits.
#[derive(Debug)]
struct KeepAllCommits;

impl IndexDeletionPolicy for KeepAllCommits {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterFromReader$1"
    }

    fn on_init(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }
}

/// Pull NRT reader after writer has committed and then indexed another doc:
/// Port of `TestIndexWriterFromReader.testAfterCommitThenIndexKeepCommits`.
fn test_after_commit_then_index_keep_commits() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = LuceneTestCase::new_index_writer_config();

    // Keep all commits:
    iwc.set_index_deletion_policy(Some(Arc::new(KeepAllCommits)))?;

    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    w.add_document(&Document::new())?;
    w.commit()?;
    w.add_document(&Document::new())?;

    let r = directory_reader::open_with_writer(&*w)?;
    assert_equals(&2, &r.max_doc());
    w.add_document(&Document::new())?;

    let r2 = directory_reader::open_with_writer(&*w)?;
    assert_equals(&3, &r2.max_doc());
    IOUtils::close([
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| w.close()),
    ])?;

    // r is not stale because, even though we've committed the original writer since it was open, we
    // are keeping all commit points:
    let iwc = iwc_from(&r)?;
    let w2 = IndexWriter::new(Arc::clone(&d), iwc)?;
    assert_equals(&2, &w2.get_doc_stats()?.max_doc);
    drop(d);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}
