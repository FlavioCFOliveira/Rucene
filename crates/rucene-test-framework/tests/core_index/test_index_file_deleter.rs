//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexFileDeleter.java`.
//!
//! Verify we can read the pre-2.1 file format, do searches
//! against it, and add documents to it.
//!
//! * `si.getCodec() instanceof SimpleTextCodec` compares the codec's name
//!   with `SimpleTextCodec`'s.
//! * The anonymous `ConcurrentMergeScheduler` that overrides
//!   `handleMergeException` is a `ConcurrentMergeScheduler` with that
//!   override installed.
//! * `catch (Throwable t)` around the writer calls catches the error the
//!   call returns.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::index_deletion_policy::{
    IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy, SnapshotDeletionPolicy,
};
use rucene::index::index_file_deleter::{self, IndexFileDeleter};
use rucene::index::index_file_names::{self, PENDING_SEGMENTS, SEGMENTS};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{MergePolicy, NoMergePolicy};
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, SerialMergeScheduler,
};
use rucene::index::segment_info::SegmentInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::default_info_stream;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::{Failure, MockDirectoryWrapper, Throttling, FAKE_IO_EXCEPTION};
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test, verbose};

/// `TestIndexFileDeleter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexFileDeleter",
        vec![
            test("testDeleteLeftoverFiles", test_delete_leftover_files),
            test(
                "testVirusScannerDoesntCorruptIndex",
                test_virus_scanner_doesnt_corrupt_index,
            ),
            test(
                "testNoSegmentsDotGenInflation",
                test_no_segments_dot_gen_inflation,
            ),
            test("testSegmentsInflation", test_segments_inflation),
            test("testSegmentNameInflation", test_segment_name_inflation),
            test("testGenerationInflation", test_generation_inflation),
            test("testTrashyFile", test_trashy_file),
            test("testTrashyGenFile", test_trashy_gen_file),
            test("testExcInDecRef", test_exc_in_dec_ref),
            test("testExcInDeleteFile", test_exc_in_delete_file),
            test(
                "testThrowExceptionWhileDeleteCommits",
                test_throw_exception_while_delete_commits,
            ),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `si.getCodec() instanceof SimpleTextCodec`.
fn is_simple_text(si: &SegmentInfo) -> bool {
    si.codec().is_some_and(|c| c.name() == "SimpleText")
}

/// Port of `TestIndexFileDeleter.testDeleteLeftoverFiles`.
fn test_delete_leftover_files() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();

    let merge_policy = LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(true, 10)?;

    // This test expects all of its segments to be in CFS
    merge_policy.set_no_cfs_ratio(1.0)?;
    merge_policy.set_max_cfs_segment_size_mb(f64::INFINITY)?;

    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(10)?;
    iwc.set_merge_policy(Some(Arc::new(merge_policy)))?;
    iwc.set_use_compound_file(true)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;

    let mut i = 0;
    while i < 35 {
        add_doc(&writer, i)?;
        i += 1;
    }
    writer.get_config().merge_policy().set_no_cfs_ratio(0.0)?;
    writer.get_config().set_use_compound_file(false);
    while i < 45 {
        add_doc(&writer, i)?;
        i += 1;
    }
    writer.close()?;

    // Delete one doc so we get a .del file:
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    iwc.set_use_compound_file(true)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    let search_term = Term::from_text("id", "7");
    writer.delete_documents_by_terms(vec![search_term])?;
    writer.close()?;

    // read in index to try to not depend on codec-specific filenames so much
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    let si0 = &sis.info(0).info;
    let si1 = &sis.info(1).info;
    let si3 = &sis.info(3).info;

    // Now, artificially create an extra .del file & extra
    // .s0 file:
    let mut files = d.list_all()?;

    // TODO: fix this test better
    let ext = ".liv";

    // Create a bogus separate del file for a
    // segment that already has a separate del file:
    copy_file(&*d, &format!("_0_1{ext}"), &format!("_0_2{ext}"))?;

    // Create a bogus separate del file for a
    // segment that does not yet have a separate del file:
    copy_file(&*d, &format!("_0_1{ext}"), &format!("_1_1{ext}"))?;

    // Create a bogus separate del file for a
    // non-existent segment:
    copy_file(&*d, &format!("_0_1{ext}"), &format!("_188_1{ext}"))?;

    let cfs_files0: &[&str] = if is_simple_text(si0) {
        &["_0.scf"]
    } else {
        &["_0.cfs", "_0.cfe"]
    };

    // Create a bogus segment file:
    copy_file(&*d, cfs_files0[0], "_188.cfs")?;

    // Create a bogus fnm file when the CFS already exists:
    copy_file(&*d, cfs_files0[0], "_0.fnm")?;

    // Create a bogus cfs file shadowing a non-cfs segment:

    // TODO: assert is bogus (relies upon codec-specific filenames)
    Assert::assert_true(
        LuceneTestCase::slow_file_exists(&*d, "_3.fdt")?
            || LuceneTestCase::slow_file_exists(&*d, "_3.fld")?,
    );

    let cfs_files3: &[&str] = if is_simple_text(si3) {
        &["_3.scf"]
    } else {
        &["_3.cfs", "_3.cfe"]
    };
    for f in cfs_files3 {
        Assert::assert_false(LuceneTestCase::slow_file_exists(&*d, f)?);
    }

    let cfs_files1: &[&str] = if is_simple_text(si1) {
        &["_1.scf"]
    } else {
        &["_1.cfs", "_1.cfe"]
    };
    copy_file(&*d, cfs_files1[0], "_3.cfs")?;

    let files_pre = d.list_all()?;

    // Open & close a writer: it should delete the above files and nothing more:
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(&d), iwc)?;
    writer.close()?;

    let mut files2 = d.list_all()?;
    drop(d);
    dir.close()?;

    files.sort();
    files2.sort();

    let dif = dif_files(&files, &files2);

    if files != files2 {
        Assert::fail(&format!(
            "IndexFileDeleter failed to delete unreferenced extra files: should have deleted {} files but only deleted {}; expected files:\n    {}\n  actual files:\n    {}\ndiff: {:?}",
            files_pre.len() as i64 - files.len() as i64,
            files_pre.len() as i64 - files2.len() as i64,
            as_string(&files),
            as_string(&files2),
            dif
        ));
    }
    Ok(())
}

/// `difFiles(files1, files2)`.
fn dif_files(files1: &[String], files2: &[String]) -> HashSet<String> {
    let set1: HashSet<&String> = files1.iter().collect();
    let set2: HashSet<&String> = files2.iter().collect();
    let mut extra = HashSet::new();
    for o in &set1 {
        if !set2.contains(o) {
            extra.insert((*o).clone());
        }
    }
    for o in &set2 {
        if !set1.contains(o) {
            extra.insert((*o).clone());
        }
    }
    extra
}

/// `asString(l)`.
fn as_string(l: &[String]) -> String {
    l.join("\n    ")
}

/// `copyFile(dir, src, dest)`.
fn copy_file(dir: &dyn Directory, src: &str, dest: &str) -> Result<()> {
    let mut r#in = dir.open_input(src, &*LuceneTestCase::new_io_context(&*random()))?;
    let mut out = dir.create_output(dest, &*LuceneTestCase::new_io_context(&*random()))?;
    let mut b = [0u8; 1024];
    let mut remainder = r#in.length();
    while remainder > 0 {
        let len = (b.len() as i64).min(remainder) as usize;
        r#in.read_bytes(Some(&mut b), 0, len)?;
        out.write_bytes(&b, 0, len)?;
        remainder -= len as i64;
    }
    r#in.close()?;
    out.close()
}

/// `addDoc(writer, id)`.
fn add_doc(writer: &IndexWriter, id: i32) -> Result<()> {
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

/// Port of `TestIndexFileDeleter.testVirusScannerDoesntCorruptIndex`.
fn test_virus_scanner_doesnt_corrupt_index() -> Result<()> {
    let path = LuceneTestCase::create_temp_dir();
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::add_virus_checker(&path));
    let d = dir.as_directory();
    TestUtil::disable_virus_checker(&*d)?;

    // add empty commit
    IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?.close()?;
    // add a trash unreferenced file
    d.create_output("_0.si", &*DEFAULT_IO_CONTEXT)?.close()?;

    // start virus scanner
    TestUtil::enable_virus_checker(&*d)?;

    let iw = IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?;
    iw.add_document(&Document::new())?;
    // stop virus scanner
    TestUtil::disable_virus_checker(&*d)?;
    iw.commit()?;
    iw.close()?;
    drop((iw, d));
    dir.close()
}

/// A `MockDirectoryWrapper` from `newMockDirectory()`, as the owner the test
/// closes.
fn mock_directory() -> TestDirectory {
    TestDirectory::Mock(LuceneTestCase::new_mock_directory())
}

fn mock(dir: &TestDirectory) -> &MockDirectoryWrapper {
    dir.as_mock().unwrap_or_else(|| crate::support::npe("dir"))
}

/// `inflateGens(sis, files, stream)`: IFD's inflater is "raw" and expects to
/// only see codec files, and rightfully so, it filters them out.
fn inflate_gens(sis: &mut SegmentInfos, files: &[String]) -> Result<()> {
    let mut filtered = HashSet::new();
    for file in files {
        if index_file_names::is_codec_file(file)
            || file.starts_with(SEGMENTS)
            || file.starts_with(PENDING_SEGMENTS)
        {
            filtered.insert(file.clone());
        }
    }
    index_file_deleter::inflate_gens(sis, &filtered, &*default_info_stream())
}

/// Port of `TestIndexFileDeleter.testNoSegmentsDotGenInflation`.
fn test_no_segments_dot_gen_inflation() -> Result<()> {
    let dir = mock_directory();
    let d = dir.as_directory();

    // empty commit
    IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?.close()?;

    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&1, &sis.generation());

    // no inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&1, &sis.generation());

    drop(d);
    dir.close()
}

/// Port of `TestIndexFileDeleter.testSegmentsInflation`.
fn test_segments_inflation() -> Result<()> {
    let dir = mock_directory();
    mock(&dir).set_check_index_on_close(false); // TODO: allow falling back more than one commit
    let d = dir.as_directory();

    // empty commit
    IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?.close()?;

    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&1, &sis.generation());

    // add trash commit
    d.create_output(&format!("{SEGMENTS}_2"), &*DEFAULT_IO_CONTEXT)?
        .close()?;

    // ensure inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&2, &sis.generation());

    // add another trash commit
    d.create_output(&format!("{SEGMENTS}_4"), &*DEFAULT_IO_CONTEXT)?
        .close()?;
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&4, &sis.generation());

    drop(d);
    dir.close()
}

/// Port of `TestIndexFileDeleter.testSegmentNameInflation`.
fn test_segment_name_inflation() -> Result<()> {
    let dir = mock_directory();
    let d = dir.as_directory();

    // empty commit
    IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?.close()?;

    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&0, &sis.counter);

    // no inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&0, &sis.counter);

    // add trash per-segment file
    d.create_output(
        &index_file_names::segment_file_name("_0", "", "foo"),
        &*DEFAULT_IO_CONTEXT,
    )?
    .close()?;

    // ensure inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&1, &sis.counter);

    // add trash per-segment file
    d.create_output(
        &index_file_names::segment_file_name("_3", "", "foo"),
        &*DEFAULT_IO_CONTEXT,
    )?
    .close()?;
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&4, &sis.counter);

    // ensure we write _4 segment next
    let iw = IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    iw.close()?;
    let sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals("_4", sis.info(0).info.name.as_str());
    assert_equals(&5, &sis.counter);

    drop((iw, d));
    dir.close()
}

/// Port of `TestIndexFileDeleter.testGenerationInflation`.
fn test_generation_inflation() -> Result<()> {
    let dir = mock_directory();
    let d = dir.as_directory();

    // initial commit
    let iw = IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    iw.close()?;

    // no deletes: start at 1
    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&1, &sis.info(0).get_next_del_gen());

    // no inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&1, &sis.info(0).get_next_del_gen());

    // add trash per-segment deletes file
    d.create_output(
        &index_file_names::file_name_from_generation("_0", "del", 2)
            .unwrap_or_else(|| crate::support::npe("fileName")),
        &*DEFAULT_IO_CONTEXT,
    )?
    .close()?;

    // ensure inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&3, &sis.info(0).get_next_del_gen());

    drop((iw, d));
    dir.close()
}

/// Port of `TestIndexFileDeleter.testTrashyFile`.
fn test_trashy_file() -> Result<()> {
    let dir = mock_directory();
    mock(&dir).set_check_index_on_close(false); // TODO: maybe handle such trash better elsewhere...
    let d = dir.as_directory();

    // empty commit
    IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?.close()?;

    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&1, &sis.generation());

    // add trash file
    d.create_output(&format!("{SEGMENTS}_"), &*DEFAULT_IO_CONTEXT)?
        .close()?;

    // no inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&1, &sis.generation());

    drop(d);
    dir.close()
}

/// Port of `TestIndexFileDeleter.testTrashyGenFile`.
fn test_trashy_gen_file() -> Result<()> {
    let dir = mock_directory();
    let d = dir.as_directory();

    // initial commit
    let iw = IndexWriter::new(Arc::clone(&d), IndexWriterConfig::with_analyzer(None))?;
    iw.add_document(&Document::new())?;
    iw.commit()?;
    iw.close()?;

    // no deletes: start at 1
    let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&d))?;
    assert_equals(&1, &sis.info(0).get_next_del_gen());

    // add trash file
    d.create_output("_1_A", &*DEFAULT_IO_CONTEXT)?.close()?;

    // no inflation
    inflate_gens(&mut sis, &d.list_all()?)?;
    assert_equals(&1, &sis.info(0).get_next_del_gen());

    drop((iw, d));
    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of `testExcInDecRef`.
struct DecRefFailure {
    do_fail_exc: Arc<AtomicBool>,
    do_fail: AtomicBool,
}

impl Failure for DecRefFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail_exc.load(Ordering::SeqCst)
            && random().next_int_bound(4) == 1
            && LuceneTestCase::call_stack_contains_method::<IndexFileDeleter>("dec_ref")
        {
            return Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                "fake fail",
            ));
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// `t.toString().contains("fake fail") || (t.getCause() != null &&
/// t.getCause().toString().contains("fake fail"))`.
fn is_fake_fail(e: &LuceneError) -> bool {
    let t = java_throwable(e);
    t.to_string().contains("fake fail")
        || t.cause()
            .is_some_and(|c| c.to_string().contains("fake fail"))
}

/// LUCENE-5919
/// Port of `TestIndexFileDeleter.testExcInDecRef`.
fn test_exc_in_dec_ref() -> Result<()> {
    let dir = mock_directory();
    let m = mock(&dir);

    // disable slow things: we don't rely upon sleeps here.
    m.set_throttling(Throttling::Never);
    m.set_use_slow_open_closers(false);

    let do_fail_exc = Arc::new(AtomicBool::new(false));

    m.fail_on(Arc::new(DecRefFailure {
        do_fail_exc: Arc::clone(&do_fail_exc),
        do_fail: AtomicBool::new(false),
    }) as Arc<dyn Failure>);

    let iwc = mock_iwc();
    // iwc.setMergeScheduler(new SerialMergeScheduler());
    let ms = iwc.merge_scheduler();
    if let Some(cms) = ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
    {
        let suppress_fake_fail = ConcurrentMergeScheduler::new();
        suppress_fake_fail.set_handle_merge_exception_override(Some(Arc::new(
            |exc: LuceneError| {
                let t = java_throwable(&exc);
                // suppress only FakeIOException:
                if t.is_instance("java.lang.RuntimeException") && t.message() == Some("fake fail") {
                    // ok to ignore
                } else if t.is_instance("java.lang.IllegalStateException")
                    && t.cause().is_some_and(|c| c.message() == Some("fake fail"))
                {
                    // also ok to ignore
                } else {
                    ConcurrentMergeScheduler::default_handle_merge_exception(exc);
                }
            },
        )));
        suppress_fake_fail
            .set_max_merges_and_threads(cms.get_max_merge_count(), cms.get_max_thread_count())?;
        iwc.set_merge_scheduler(Some(Arc::new(suppress_fake_fail) as Arc<dyn MergeScheduler>))?;
    }

    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    // Since we hit exc during merging, a partial
    // forceMerge can easily return when there are still
    // too many segments in the index:
    w.set_do_random_force_merge_assert(false);

    do_fail_exc.store(true, Ordering::SeqCst);
    let iters = LuceneTestCase::at_least(1000);
    for _ in 0..iters {
        let result = (|| -> Result<()> {
            if random().next_int_bound(10) == 5 {
                w.commit()?;
            } else if random().next_int_bound(10) == 7 {
                w.get_reader()?.close()?;
            } else {
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "field",
                    "some text",
                    Store::NO,
                )));
                w.add_document(&doc)?;
            }
            Ok(())
        })();
        if let Err(t) = result {
            if is_fake_fail(&t) {
                // ok
            } else {
                return Err(t);
            }
        }
    }

    do_fail_exc.store(false, Ordering::SeqCst);
    w.close()?;
    drop(w);
    dir.close()
}

/// The anonymous `MockDirectoryWrapper.Failure` of `testExcInDeleteFile`.
struct DeleteFileFailure {
    do_fail_exc: Arc<AtomicBool>,
    do_fail: AtomicBool,
}

impl Failure for DeleteFileFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail_exc.load(Ordering::SeqCst)
            && random().next_int_bound(4) == 1
            && LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("delete_file")
        {
            return Err(rucene_test_framework::store::fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// LUCENE-6835: make sure best-effort to not create an "apparently but not really" corrupt index
/// is working:
/// Port of `TestIndexFileDeleter.testExcInDeleteFile`.
fn test_exc_in_delete_file() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for iter in 0..iters {
        if verbose() {
            println!("TEST: iter={iter}");
        }
        let dir = mock_directory();

        let do_fail_exc = Arc::new(AtomicBool::new(false));

        mock(&dir).fail_on(Arc::new(DeleteFileFailure {
            do_fail_exc: Arc::clone(&do_fail_exc),
            do_fail: AtomicBool::new(false),
        }) as Arc<dyn Failure>);

        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        w.add_document(&Document::new())?;

        // makes segments_1
        if verbose() {
            println!("TEST: now commit");
        }
        w.commit()?;

        w.add_document(&Document::new())?;
        do_fail_exc.store(true, Ordering::SeqCst);
        if verbose() {
            println!("TEST: now close");
        }
        match w.close() {
            Ok(()) => {
                if verbose() {
                    println!("TEST: no exception (ok)");
                }
            }
            Err(e)
                if is_instance(&e, "java.lang.RuntimeException")
                    && !is_instance(&e, FAKE_IO_EXCEPTION) =>
            {
                Assert::assert_true(
                    java_throwable(&e)
                        .cause()
                        .is_some_and(|c| c.is_instance(FAKE_IO_EXCEPTION)),
                );
                // good
                if verbose() {
                    println!("TEST: got expected exception:");
                    println!("{}", java_throwable(&e));
                }
            }
            Err(e) if is_instance(&e, FAKE_IO_EXCEPTION) => {
                // good
                if verbose() {
                    println!("TEST: got expected exception:");
                    println!("{}", java_throwable(&e));
                }
            }
            Err(e) => return Err(e),
        }
        do_fail_exc.store(false, Ordering::SeqCst);
        Assert::assert_false(w.w().is_open());

        let d = dir.as_directory();
        for name in d.list_all()? {
            if name.starts_with(SEGMENTS) {
                if verbose() {
                    println!("TEST: now read {name}");
                }
                SegmentInfos::read_commit(Arc::clone(&d), &name)?;
            }
        }
        drop((w, d));
        dir.close()?;
    }
    Ok(())
}

/// The anonymous `MockDirectoryWrapper.Failure` of
/// `testThrowExceptionWhileDeleteCommits`.
struct DeleteCommitsFailure {
    fail_on_delete_commits: Arc<AtomicBool>,
    do_fail: AtomicBool,
}

impl Failure for DeleteCommitsFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.fail_on_delete_commits.load(Ordering::SeqCst)
            && LuceneTestCase::call_stack_contains_method::<IndexFileDeleter>("delete_commits")
            && LuceneTestCase::call_stack_contains_method::<MockDirectoryWrapper>("delete_file")
            && random().next_int_bound(4) == 1
        {
            return Err(rucene_test_framework::store::fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexFileDeleter.testThrowExceptionWhileDeleteCommits`.
fn test_throw_exception_while_delete_commits() -> Result<()> {
    let dir = mock_directory();
    let result = (|| -> Result<()> {
        let fail_on_delete_commits = Arc::new(AtomicBool::new(false));
        mock(&dir).fail_on(Arc::new(DeleteCommitsFailure {
            fail_on_delete_commits: Arc::clone(&fail_on_delete_commits),
            do_fail: AtomicBool::new(false),
        }) as Arc<dyn Failure>);
        let snapshot_deletion_policy = Arc::new(SnapshotDeletionPolicy::new(Some(Arc::new(
            KeepOnlyLastCommitDeletionPolicy::new(),
        ))));
        let config = LuceneTestCase::new_index_writer_config();
        config.set_index_deletion_policy(Some(
            Arc::clone(&snapshot_deletion_policy) as Arc<dyn IndexDeletionPolicy>
        ))?;

        let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
        let inner = (|| -> Result<()> {
            writer.add_document(&Document::new())?;
            writer.commit()?;

            let snapshot_commit = snapshot_deletion_policy.snapshot()?;
            let commits = TestUtil::next_int(&*random(), 1, 3);
            for _ in 0..commits {
                writer.add_document(&Document::new())?;
                writer.commit()?;
            }
            snapshot_deletion_policy.release(&*snapshot_commit)?;
            fail_on_delete_commits.store(true, Ordering::SeqCst);
            if let Err(e) = writer.w().delete_unused_files() {
                if !is_instance(&e, "java.io.IOException") {
                    return Err(e);
                }
                Assert::assert_true(is_instance(&e, FAKE_IO_EXCEPTION));
            }
            fail_on_delete_commits.store(false, Ordering::SeqCst);
            for _ in 0..commits {
                writer.add_document(&Document::new())?;
                writer.commit()?;
            }
            Ok(())
        })();
        let closed = writer.close();
        inner?;
        closed
    })();
    let closed = dir.close();
    result?;
    closed
}
