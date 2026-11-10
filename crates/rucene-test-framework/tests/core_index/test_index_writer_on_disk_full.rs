//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterOnDiskFull.java`.
//!
//! Tests for IndexWriter when the disk runs out of space
//!
//! * `callStackContains(LiveDocsFormat.class, "writeLiveDocs")` names the
//!   abstract class, whose frame never appears in Java (the frame is the
//!   concrete format's); `call_stack_contains_method::<dyn LiveDocsFormat>`
//!   never matches either.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::LiveDocsFormat;
use rucene::document::{Document, FieldType, IntPoint, NumericDocValuesField, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
use rucene::index::segment_merger::SegmentMerger;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::store::{Failure, MockDirectoryWrapper};
use rucene_test_framework::util::java_exceptions::{expect_throws, is_instance};
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals_msg, class, random, test, verbose};
use crate::support::KeepFullyDeletedSegmentsMergePolicy;

const IOE: &str = "java.io.IOException";
const ISE: &str = "java.lang.IllegalStateException";
const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const MERGE_EXCEPTION: &str = "org.apache.lucene.index.MergePolicy$MergeException";

/// `TestIndexWriterOnDiskFull`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterOnDiskFull",
        vec![
            test("testAddDocumentOnDiskFull", test_add_document_on_disk_full),
            test("testAddIndexOnDiskFull", test_add_index_on_disk_full),
            test(
                "testCorruptionAfterDiskFullDuringMerge",
                test_corruption_after_disk_full_during_merge,
            ),
            test("testImmediateDiskFull", test_immediate_disk_full),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `ms instanceof ConcurrentMergeScheduler ? (ConcurrentMergeScheduler) ms : null`.
fn as_cms_opt(
    ms: &Arc<dyn rucene::index::merge_scheduler::MergeScheduler>,
) -> Option<&ConcurrentMergeScheduler> {
    ms.as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
}

/// Make sure IndexWriter cleans up on hitting a disk
/// full exception in addDocument.
/// TODO: how to do this on windows with FSDirectory?
///
/// Port of `TestIndexWriterOnDiskFull.testAddDocumentOnDiskFull`.
fn test_add_document_on_disk_full() -> Result<()> {
    for pass in 0..2 {
        if verbose() {
            println!("TEST: pass={pass}");
        }
        let do_abort = pass == 1;
        let mut disk_free = TestUtil::next_int(&*random(), 100, 300) as i64;
        let mut index_exists = false;
        loop {
            if verbose() {
                println!("TEST: cycle: diskFree={disk_free}");
            }
            let mut dir =
                MockDirectoryWrapper::new(&*random(), Arc::new(ByteBuffersDirectory::new()));
            dir.set_max_size_in_bytes(disk_free);
            let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
            let ms = writer.get_config().merge_scheduler();
            if let Some(cms) = as_cms_opt(&ms) {
                // This test intentionally produces exceptions
                // in the threads that CMS launches; we don't
                // want to pollute test output with these.
                cms.set_suppress_exceptions();
            }

            let mut hit_error = false;
            let attempt = (|| -> Result<()> {
                for _ in 0..200 {
                    add_doc(&writer)?;
                }
                if verbose() {
                    println!("TEST: done adding docs; now commit");
                }
                // when calling commit(), if the writer is asynchronously closed
                // by a fatal tragedy (e.g. from disk-full-on-merge with CMS),
                // then we may receive either AlreadyClosedException OR IllegalStateException,
                // depending on when it happens.
                match writer.commit() {
                    Ok(_) => index_exists = true,
                    Err(e) if is_instance(&e, IOE) || is_instance(&e, ISE) => {
                        if verbose() {
                            println!("TEST: exception on commit");
                            println!("{e}");
                        }
                        hit_error = true;
                    }
                    Err(e) => return Err(e),
                }
                Ok(())
            })();
            match attempt {
                Ok(()) => {}
                Err(e) if is_instance(&e, IOE) => {
                    if verbose() {
                        println!("TEST: exception on addDoc");
                        println!("{e}");
                    }
                    hit_error = true;
                }
                Err(e) => return Err(e),
            }

            if hit_error {
                if do_abort {
                    if verbose() {
                        println!("TEST: now rollback");
                    }
                    writer.rollback()?;
                } else {
                    if verbose() {
                        println!("TEST: now close");
                    }
                    match writer.close() {
                        Ok(()) => {}
                        Err(e) if is_instance(&e, IOE) || is_instance(&e, ISE) => {
                            if verbose() {
                                println!("TEST: exception on close; retry w/ no disk space limit");
                                println!("{e}");
                            }
                            dir.set_max_size_in_bytes(0);
                            match writer.close() {
                                Ok(()) => {}
                                Err(ace) if is_instance(&ace, ACE) => {
                                    // OK
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }

                // _TestUtil.syncConcurrentMerges(ms);

                if index_exists {
                    // Make sure reader can open the index:
                    directory_reader::open(dir.as_directory())?.close()?;
                }

                drop(writer);
                dir.close()?;
                // Now try again w/ more space:

                disk_free += if *TEST_NIGHTLY {
                    TestUtil::next_int(&*random(), 400, 600) as i64
                } else {
                    TestUtil::next_int(&*random(), 3000, 5000) as i64
                };
            } else {
                // _TestUtil.syncConcurrentMerges(writer);
                dir.set_max_size_in_bytes(0);
                writer.close()?;
                drop(writer);
                dir.close()?;
                break;
            }
        }
    }
    Ok(())
}

// TODO: make @Nightly variant that provokes more disk
// fulls

// TODO: have test fail if on any given top
// iter there was not a single IOE hit

/// Test: make sure when we run out of disk space or hit
/// random IOExceptions in any of the addIndexes(*) calls
/// that 1) index is not corrupt (searcher can open/search
/// it) and 2) transactional semantics are followed:
/// either all or none of the incoming documents were in
/// fact added.
///
/// Port of `TestIndexWriterOnDiskFull.testAddIndexOnDiskFull`.
fn test_add_index_on_disk_full() -> Result<()> {
    // MemoryCodec, since it uses FST, is not necessarily
    // "additive", ie if you add up N small FSTs, then merge
    // them, the merged result can easily be larger than the
    // sum because the merged FST may use array encoding for
    // some arcs (which uses more space):

    let start_count: i32 = 57;
    let num_dir: i32 = if *TEST_NIGHTLY { 50 } else { 5 };
    let end_count: i32 = start_count + num_dir * if *TEST_NIGHTLY { 25 } else { 5 };

    // Build up a bunch of dirs that have indexes which we
    // will then merge together by calling addIndexes(*):
    let mut dir_holders: Vec<TestDirectory> = Vec::with_capacity(num_dir as usize);
    let mut input_disk_usage: i64 = 0;
    for i in 0..num_dir {
        let dir = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
        for j in 0..25 {
            add_doc_with_index(&writer, 25 * i + j)?;
        }
        writer.close()?;
        let files = dir.as_directory().list_all()?;
        for file in &files {
            input_disk_usage += dir.as_directory().file_length(file)?;
        }
        dir_holders.push(dir);
    }
    let dirs: Vec<Arc<dyn Directory>> = dir_holders.iter().map(|d| d.as_directory()).collect();

    // Now, build a starting index that has START_COUNT docs.  We
    // will then try to addIndexes into a copy of this:
    let mut start_dir = LuceneTestCase::new_mock_directory();
    let writer = IndexWriter::new(start_dir.as_directory(), mock_iwc())?;
    for j in 0..start_count {
        add_doc_with_index(&writer, j)?;
    }
    writer.close()?;
    drop(writer);

    // Make sure starting index seems to be working properly:
    let search_term = Term::from_text("content", "aaa");
    let reader = directory_reader::open(start_dir.as_directory())?;
    assert_equals_msg(Some("first docFreq"), &57, &reader.doc_freq(&search_term)?);

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let hits = searcher
        .search(Arc::new(TermQuery::new(search_term.clone())), 1000)?
        .score_docs;
    assert_equals_msg(Some("first number of hits"), &57, &hits.len());
    reader.close()?;

    // Iterate with larger and larger amounts of free
    // disk space.  With little free disk space,
    // addIndexes will certainly run out of space &
    // fail.  Verify that when this happens, index is
    // not corrupt and index in fact has added no
    // documents.  Then, we increase disk space by 2000
    // bytes each iteration.  At some point there is
    // enough free disk space and addIndexes should
    // succeed and index should show all documents were
    // added.

    // String[] files = startDir.listAll();
    let disk_usage = start_dir.size_in_bytes()?;

    let mut start_disk_usage: i64 = 0;
    let files = start_dir.as_directory().list_all()?;
    for file in &files {
        start_disk_usage += start_dir.as_directory().file_length(file)?;
    }

    for iter in 0..3 {
        if verbose() {
            println!("TEST: iter={iter}");
        }

        // Start with 100 bytes more than we are currently using:
        let mut disk_free = disk_usage + TestUtil::next_int(&*random(), 50, 200) as i64;

        let method = iter;

        let mut success: bool;
        let mut done = false;

        let method_name = if 0 == method {
            "addIndexes(Directory[]) + forceMerge(1)"
        } else if 1 == method {
            "addIndexes(IndexReader[])"
        } else {
            "addIndexes(Directory[])"
        };

        while !done {
            if verbose() {
                println!("TEST: cycle...");
            }

            // Make a new dir that will enforce disk usage:
            let copy = TestUtil::ram_copy_of(&*start_dir.as_directory())?;
            let mut dir = MockDirectoryWrapper::new(&*random(), Arc::from(copy));
            let iwc = mock_iwc();
            iwc.set_open_mode(OpenMode::APPEND)?;
            iwc.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
            let mut writer = IndexWriter::new(dir.as_directory(), iwc)?;
            let mut err: Option<LuceneError> = None;

            for x in 0..2 {
                let ms = writer.get_config().merge_scheduler();
                if let Some(cms) = as_cms_opt(&ms) {
                    // This test intentionally produces exceptions
                    // in the threads that CMS launches; we don't
                    // want to pollute test output with these.
                    if 0 == x {
                        cms.set_suppress_exceptions();
                    } else {
                        cms.clear_suppress_exceptions();
                    }
                }

                // Two loops: first time, limit disk space &
                // throw random IOExceptions; second time, no
                // disk space limit:

                let mut rate = 0.05;
                let disk_ratio = disk_free as f64 / disk_usage as f64;
                let this_disk_free;

                let mut test_name: Option<String> = None;

                if 0 == x {
                    dir.set_random_io_exception_rate_on_open(random().next_double() * 0.01);
                    this_disk_free = disk_free;
                    if disk_ratio >= 2.0 {
                        rate /= 2.0;
                    }
                    if disk_ratio >= 4.0 {
                        rate /= 2.0;
                    }
                    if disk_ratio >= 6.0 {
                        rate = 0.0;
                    }
                    if verbose() {
                        test_name = Some(format!(
                            "disk full test {method_name} with disk full at {disk_free} bytes"
                        ));
                    }
                } else {
                    dir.set_random_io_exception_rate_on_open(0.0);
                    this_disk_free = 0;
                    rate = 0.0;
                    if verbose() {
                        test_name = Some(format!(
                            "disk full test {method_name} with unlimited disk space"
                        ));
                    }
                }
                let test_name_s = test_name.clone().unwrap_or_else(|| "null".to_string());

                if verbose() {
                    println!("\ncycle: {test_name_s}");
                }

                dir.set_track_disk_usage(true);
                dir.set_max_size_in_bytes(this_disk_free);
                dir.set_random_io_exception_rate(rate);

                let attempt = (|| -> Result<()> {
                    if 0 == method {
                        if verbose() {
                            println!("TEST: now addIndexes count={}", dirs.len());
                        }
                        writer.add_indexes_dirs(&dirs)?;
                        if verbose() {
                            println!("TEST: now forceMerge");
                        }
                        writer.force_merge(1)?;
                    } else if 1 == method {
                        let mut readers: Vec<Arc<dyn DirectoryReader>> =
                            Vec::with_capacity(dirs.len());
                        for d in &dirs {
                            readers.push(directory_reader::open(Arc::clone(d))?);
                        }
                        let result = TestUtil::add_indexes_slowly(&writer, &readers);
                        let mut close_err = None;
                        for r in &readers {
                            if let Err(e) = r.close() {
                                close_err.get_or_insert(e);
                            }
                        }
                        result?;
                        if let Some(e) = close_err {
                            return Err(e);
                        }
                    } else {
                        writer.add_indexes_dirs(&dirs)?;
                    }
                    Ok(())
                })();
                match attempt {
                    Ok(()) => {
                        success = true;
                        if verbose() {
                            println!("  success!");
                        }

                        if 0 == x {
                            done = true;
                        }
                    }
                    Err(e)
                        if is_instance(&e, ISE)
                            || is_instance(&e, IOE)
                            || is_instance(&e, MERGE_EXCEPTION) =>
                    {
                        success = false;
                        if verbose() {
                            println!("  hit Exception: {e}");
                        }

                        if 1 == x && !is_instance(&e, MERGE_EXCEPTION) {
                            println!("{e}");
                            Assert::fail(&format!(
                                "{method_name} hit IOException after disk space was freed up"
                            ));
                        }
                        err = Some(e);
                    }
                    Err(e) => return Err(e),
                }

                if x == 1 {
                    // Make sure all threads from ConcurrentMergeScheduler are done
                    TestUtil::sync_concurrent_merges_of_writer(&writer);
                } else {
                    dir.set_random_io_exception_rate_on_open(0.0);
                    writer.rollback()?;
                    let iwc = mock_iwc();
                    iwc.set_open_mode(OpenMode::APPEND)?;
                    iwc.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
                        false,
                    )))?;
                    writer = IndexWriter::new(dir.as_directory(), iwc)?;
                }

                if verbose() {
                    println!("  now test readers");
                }

                // Finally, verify index is not corrupt, and, if
                // we succeeded, we see all docs added, and if we
                // failed, we see either all docs or no docs added
                // (transactional semantics):
                dir.set_random_io_exception_rate_on_open(0.0);
                let reader = match directory_reader::open(dir.as_directory()) {
                    Ok(r) => r,
                    Err(e) if is_instance(&e, IOE) => {
                        println!("{e}");
                        Assert::fail(&format!(
                            "{test_name_s}: exception when creating IndexReader: {e}"
                        ))
                    }
                    Err(e) => return Err(e),
                };
                let result = reader.doc_freq(&search_term)?;
                if success {
                    if result != start_count {
                        Assert::fail(&format!(
                            "{test_name_s}: method did not throw exception but docFreq('aaa') is {result} instead of expected {start_count}"
                        ));
                    }
                } else {
                    // On hitting exception we still may have added
                    // all docs:
                    if result != start_count && result != end_count {
                        if let Some(e) = &err {
                            println!("{e}");
                        }
                        Assert::fail(&format!(
                            "{test_name_s}: method did throw exception but docFreq('aaa') is {result} instead of expected {start_count} or {end_count}"
                        ));
                    }
                }

                let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
                let hits = match searcher
                    .search(Arc::new(TermQuery::new(search_term.clone())), end_count)
                {
                    Ok(top) => top.score_docs,
                    Err(e) if is_instance(&e, IOE) => {
                        println!("{e}");
                        Assert::fail(&format!("{test_name_s}: exception when searching: {e}"))
                    }
                    Err(e) => return Err(e),
                };
                let result2 = hits.len() as i32;
                if success {
                    if result2 != result {
                        Assert::fail(&format!(
                            "{test_name_s}: method did not throw exception but hits.length for search on term 'aaa' is {result2} instead of expected {result}"
                        ));
                    }
                } else {
                    // On hitting exception we still may have added
                    // all docs:
                    if result2 != result {
                        if let Some(e) = &err {
                            println!("{e}");
                        }
                        Assert::fail(&format!(
                            "{test_name_s}: method did throw exception but hits.length for search on term 'aaa' is {result2} instead of expected {result}"
                        ));
                    }
                }

                reader.close()?;
                if verbose() {
                    println!("  count is {result}");
                }

                if done || result == end_count {
                    break;
                }
            }

            if verbose() {
                println!(
                    "  start disk = {start_disk_usage}; input disk = {input_disk_usage}; max used = {}",
                    dir.get_max_used_size_in_bytes()
                );
            }

            if done {
                // Javadocs state that temp free Directory space
                // required is at most 2X total input size of
                // indices so let's make sure:
                Assert::assert_true_msg(
                    &format!(
                        "max free Directory space required exceeded 1X the total input index sizes during {method_name}: max temp usage = {} bytes vs limit={}; starting disk usage = {start_disk_usage} bytes; input index disk usage = {input_disk_usage} bytes",
                        dir.get_max_used_size_in_bytes() - start_disk_usage,
                        2 * (start_disk_usage + input_disk_usage)
                    ),
                    (dir.get_max_used_size_in_bytes() - start_disk_usage)
                        < 2 * (start_disk_usage + input_disk_usage),
                );
            }

            // Make sure we don't hit disk full during close below:
            dir.set_max_size_in_bytes(0);
            dir.set_random_io_exception_rate(0.0);
            dir.set_random_io_exception_rate_on_open(0.0);

            writer.close()?;
            drop(writer);

            dir.close()?;

            // Try again with more free space:
            disk_free += if *TEST_NIGHTLY {
                TestUtil::next_int(&*random(), 4000, 8000) as i64
            } else {
                TestUtil::next_int(&*random(), 40000, 80000) as i64
            };
        }
    }

    start_dir.close()?;
    drop(dirs);
    for dir in &mut dir_holders {
        dir.close()?;
    }
    Ok(())
}

/// `FailTwiceDuringMerge`.
#[derive(Default)]
struct FailTwiceDuringMerge {
    do_fail: AtomicBool,
    did_fail1: AtomicBool,
    did_fail2: AtomicBool,
}

impl Failure for FailTwiceDuringMerge {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if !self.do_fail() {
            return Ok(());
        }
        if LuceneTestCase::call_stack_contains_method::<SegmentMerger>("merge_terms")
            && !self.did_fail1.load(Ordering::SeqCst)
        {
            self.did_fail1.store(true, Ordering::SeqCst);
            return Err(LuceneError::Io(std::io::Error::other(
                "fake disk full during mergeTerms",
            )));
        }
        if LuceneTestCase::call_stack_contains_method::<dyn LiveDocsFormat>("write_live_docs")
            && !self.did_fail2.load(Ordering::SeqCst)
        {
            self.did_fail2.store(true, Ordering::SeqCst);
            return Err(LuceneError::Io(std::io::Error::other(
                "fake disk full while writing LiveDocs",
            )));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// LUCENE-2593
///
/// Port of `TestIndexWriterOnDiskFull.testCorruptionAfterDiskFullDuringMerge`.
fn test_corruption_after_disk_full_during_merge() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    // IndexWriter w = new IndexWriter(dir, newIndexWriterConfig(new
    // MockAnalyzer(random)).setReaderPooling(true));
    let mp = LogMergePolicy::by_doc_count();
    mp.set_merge_factor(2)?;
    let iwc = mock_iwc();
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_reader_pooling(true);
    // we can do this because we add/delete/add (and dont merge to "nothing")
    iwc.set_merge_policy(Some(Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(
        Arc::new(mp),
    ))))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();

    doc.add(Box::new(LuceneTestCase::new_text_field(
        "f",
        "doctor who",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.commit()?;

    w.delete_documents_by_terms(vec![Term::from_text("f", "who")])?;
    w.add_document(&doc)?;

    // disk fills up!
    let ftdm = Arc::new(FailTwiceDuringMerge::default());
    ftdm.set_do_fail();
    dir.fail_on(Arc::clone(&ftdm) as Arc<dyn Failure>);

    expect_throws(IOE, || w.commit());
    Assert::assert_true(
        ftdm.did_fail1.load(Ordering::SeqCst) || ftdm.did_fail2.load(Ordering::SeqCst),
    );

    TestUtil::check_index(dir.as_directory())?;
    ftdm.clear_do_fail();
    expect_throws(ACE, || w.add_document(&doc));

    drop(w);
    dir.close()
}

/// LUCENE-1130: make sure immediate disk full on creating
/// an IndexWriter (hit during DWPT#updateDocuments()) is
/// OK:
///
/// Port of `TestIndexWriterOnDiskFull.testImmediateDiskFull`.
fn test_immediate_disk_full() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_commit_on_close(false);
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.commit()?; // empty commit, to not create confusing situation with first commit
    dir.set_max_size_in_bytes(1.max(dir.size_in_bytes()?));
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(TextField::type_stored());
    doc.add(Box::new(LuceneTestCase::new_field(
        "field",
        "aaa bbb ccc ddd eee fff ggg hhh iii jjj",
        &custom_type,
    )));
    expect_throws(IOE, || writer.add_document(&doc));
    Assert::assert_true(writer.is_deleter_closed()?);
    Assert::assert_true(writer.is_closed());

    drop(writer);
    dir.close()
}

// TODO: these are also in TestIndexWriter... add a simple doc-writing method
// like this to LuceneTestCase?
/// The private `addDoc(IndexWriter)`.
fn add_doc(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        "aaa",
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("numericdv", 1)));
    doc.add(Box::new(IntPoint::new("point", &[1])?));
    doc.add(Box::new(IntPoint::new("point2d", &[1, 1])?));
    writer.add_document(&doc)?;
    Ok(())
}

/// The private `addDocWithIndex(IndexWriter, int)`.
fn add_doc_with_index(writer: &IndexWriter, index: i32) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        &format!("aaa {index}"),
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "id",
        &index.to_string(),
        Store::NO,
    )));
    doc.add(Box::new(NumericDocValuesField::new("numericdv", 1)));
    doc.add(Box::new(IntPoint::new("point", &[1])?));
    doc.add(Box::new(IntPoint::new("point2d", &[1, 1])?));
    writer.add_document(&doc)?;
    Ok(())
}
