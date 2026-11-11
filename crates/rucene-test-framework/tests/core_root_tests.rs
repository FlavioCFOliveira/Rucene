//! Port of the test classes of Lucene 10.5.0's root test package
//! (`lucene/core/src/test/org/apache/lucene/`), each run as a suite by the
//! ported runner: `TestAssertions`, `TestDemo`, `TestExternalCodecs`,
//! `TestMergeSchedulerExternal`, `TestSearchForDuplicates` and `TestSearch`.

mod lucene_core_shims;
mod suite_seeds;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lucene_core_shims::{lucene_class, run, t};
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{After, Assert};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{Document, Field, NumericDocValuesField, Store, StoredField};
use rucene::error::Result;
use rucene::error::{LuceneError, LuceneErrorKind};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::index_writer_config::LiveIndexWriterConfig;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_policy::MergeTrigger;
use rucene::index::merge_scheduler::{
    ConcurrentMergeScheduler, MergeScheduler, MergeSource, MergeThread,
};
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::field_doc::FieldDoc;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene::store::{Directory, FSDirectory};
use rucene::util::attribute::AttributeSource;
use rucene::util::concurrent::SameThreadExecutorService;
use rucene::util::java_fmt::float_to_string;
use rucene::util::PrintStreamInfoStream;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::AssertingCodec;
use rucene_test_framework::store::mock_directory_wrapper::Failure;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::java_exceptions::is_instance;
use rucene_test_framework::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use rucene_test_framework::util::TestUtil;

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

// -----------------------------------------------------------------------------
// TestAssertions
// -----------------------------------------------------------------------------

/// `TestTokenStream1` (`static class … extends TokenStream`, with a `final`
/// `incrementToken()`).
#[derive(Debug)]
struct TestTokenStream1 {
    source: AttributeSource,
}

/// `TestTokenStream2` (`static final class … extends TokenStream`).
#[derive(Debug)]
struct TestTokenStream2 {
    source: AttributeSource,
}

/// `TestTokenStream3` (`static class … extends TokenStream`, neither the
/// class nor `incrementToken()` `final`).
#[derive(Debug)]
struct TestTokenStream3 {
    source: AttributeSource,
}

macro_rules! test_token_stream {
    ($name:ident) => {
        impl $name {
            fn new() -> Self {
                $name {
                    source: AttributeSource::new(),
                }
            }
        }

        impl TokenStream for $name {
            fn increment_token(&mut self) -> Result<bool> {
                Ok(false)
            }

            fn attribute_source(&self) -> &AttributeSource {
                &self.source
            }

            fn attribute_source_mut(&mut self) -> &mut AttributeSource {
                &mut self.source
            }
        }
    };
}

test_token_stream!(TestTokenStream1);
test_token_stream!(TestTokenStream2);
test_token_stream!(TestTokenStream3);

/// `TestAssertions.testTokenStreams()`.
///
/// Java's `TokenStream` constructors `assert assertFinal()`, so constructing
/// `TestTokenStream3` throws `AssertionError` when assertions are enabled.
/// Rucene's `TokenStream` declares that check unportable (see the trait's
/// docs): no `AssertionError` is raised here, and with assertions enabled
/// the test reaches `fail(…)`, as the translation of Java's body does.
fn assertions_test_token_streams() -> Result<()> {
    let _ = TestTokenStream1::new();
    let _ = TestTokenStream2::new();
    // `try { … } catch (AssertionError e) { // expected }`
    let caught = std::panic::catch_unwind(|| {
        let _ = TestTokenStream3::new();
        if LuceneTestCase::ASSERTS_ARE_ENABLED {
            Assert::fail("TestTokenStream3 should fail assertion");
        }
    });
    if let Err(payload) = caught {
        let is_assertion_error = payload
            .downcast_ref::<randomizedtesting::jdk::exception::Throwable>()
            .is_some_and(|t| t.is_instance("java.lang.AssertionError"));
        if !is_assertion_error {
            std::panic::resume_unwind(payload);
        }
        // expected
    }
    Ok(())
}

fn assertions_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestAssertions")
        .method(t("testTokenStreams", assertions_test_token_streams))
        .build()
}

// -----------------------------------------------------------------------------
// TestDemo
// -----------------------------------------------------------------------------

/// `TestDemo.testDemo()`: a very simple demo used in the API documentation
/// (`src/java/overview.html`).
///
/// **Divergence from Lucene 10.5.0.** `Files.createTempDirectory("tempIndex")`
/// and `IOUtils.rm(indexPath)` have no port: the directory is created under
/// the JVM's `java.io.tmpdir` equivalent (`std::env::temp_dir()`) with the
/// same prefix, and removed recursively at the end.
fn demo_test_demo() -> Result<()> {
    let long_term = concat!(
        "longtermlongtermlongtermlongtermlongtermlongtermlongtermlong",
        "termlongtermlongtermlongtermlongtermlongtermlongtermlongterm",
        "longtermlongtermlongterm"
    );
    let text = format!("This is the text to be indexed. {long_term}");

    let index_path = create_temp_directory("tempIndex")?;
    {
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(index_path.as_path())?);
        let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
        {
            let iw = IndexWriter::new(
                Arc::clone(&dir),
                IndexWriterConfig::with_analyzer(Some(analyzer)),
            )?;
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "fieldname",
                &text,
                Store::YES,
            )));
            iw.add_document(&doc)?;
            iw.close()?;
        }

        // Now search the index.
        {
            let reader = directory_reader::open(Arc::clone(&dir))?;
            let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

            Assert::assert_equals(
                1,
                searcher.count(Arc::new(TermQuery::new(Term::from_text(
                    "fieldname",
                    long_term,
                ))))?,
            );

            let query: Arc<dyn Query> =
                Arc::new(TermQuery::new(Term::from_text("fieldname", "text")));
            let hits = searcher.search(query, 1)?;
            Assert::assert_equals(1i64, hits.total_hits.value());

            // Iterate through the results.
            let stored_fields = searcher.stored_fields()?;
            for hit in &hits.score_docs {
                let hit_doc = stored_fields.document(hit.doc)?;
                Assert::assert_equals_str(None, Some(&text), hit_doc.get("fieldname").as_deref());
            }

            // Test simple phrase query.
            let phrase_query = PhraseQuery::from_strings("fieldname", &["to", "be"])?;
            Assert::assert_equals(1, searcher.count(Arc::new(phrase_query))?);
            reader.close()?;
        }
        close_arc_directory(dir)?;
    }
    std::fs::remove_dir_all(&index_path)?;
    Ok(())
}

/// `Files.createTempDirectory(prefix)`.
fn create_temp_directory(prefix: &str) -> Result<std::path::PathBuf> {
    let base = std::env::temp_dir();
    for i in 0u64.. {
        let candidate = base.join(format!(
            "{prefix}{}{i}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}

/// `dir.close()` on a directory the test shares as an `Arc`.
fn close_arc_directory(dir: Arc<dyn Directory>) -> Result<()> {
    rucene_test_framework::store::base_directory_test_case::close_shared(dir)
}

fn demo_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestDemo")
        .method(t("testDemo", demo_test_demo))
        .build()
}

// -----------------------------------------------------------------------------
// TestExternalCodecs
// -----------------------------------------------------------------------------

/// `BaseDirectoryWrapper.setCheckIndexOnClose(boolean)` on the directory
/// `newDirectory()` returned.
fn set_check_index_on_close(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    value: bool,
) {
    use rucene_test_framework::util::lucene_test_case::TestDirectory;
    match dir {
        TestDirectory::Mock(m) => m.set_check_index_on_close(value),
        TestDirectory::Raw(r) => r.base().set_check_index_on_close(value),
    }
}

/// `CustomPerFieldCodec` (`private static final class … extends
/// AssertingCodec`): "id", "field1" and "field2" use the default postings
/// format, every other field `RAMOnly`.
fn custom_per_field_codec() -> AssertingCodec {
    let ram_format = rucene::codecs::postings_for_name("RAMOnly").expect("RAMOnly resolves");
    let default_format = TestUtil::get_default_postings_format();
    AssertingCodec::with_formats_for_field(
        Some(Arc::new(move |field: &str| {
            if field == "field2" || field == "field1" || field == "id" {
                Arc::clone(&default_format)
            } else {
                Arc::clone(&ram_format)
            }
        })),
        None,
        None,
    )
}

/// `TestExternalCodecs.testPerFieldCodec()`: tests storing "id" and
/// "field2" fields as pulsing codec, whose term sort is backwards unicode
/// code point, and storing "field1" as a custom entirely-in-RAM codec.
fn external_codecs_test_per_field_codec() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(173);
    if *VERBOSE {
        println!("TEST: NUM_DOCS={num_docs}");
    }

    let mut dir = LuceneTestCase::new_directory();
    set_check_index_on_close(&dir, false); // we use a custom codec provider
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    config.set_codec(Arc::new(custom_per_field_codec()))?;
    config.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(3)?,
    )))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let mut doc = Document::new();
    // uses default codec:
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field1",
        "this field uses the standard codec as the test",
        Store::NO,
    )));
    // uses memory codec:
    let field2 = LuceneTestCase::new_text_field(
        "field2",
        "this field uses the memory codec as the test",
        Store::NO,
    );
    doc.add(Box::new(field2));

    let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
    doc.add(Box::new(id_field));

    for i in 0..num_docs {
        doc.get_field_mut_as::<Field>("id")
            .expect("idField")
            .set_string_value(i.to_string())?;
        w.add_document(&doc)?;
        if (i + 1) % 10 == 0 {
            w.commit()?;
        }
    }
    if *VERBOSE {
        println!("TEST: now delete id=77");
    }
    w.delete_documents_by_terms(vec![Term::from_text("id", "77")])?;

    let r = directory_reader::open_with_writer(&*w)?;

    Assert::assert_equals(num_docs - 1, r.num_docs());
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    Assert::assert_equals(
        num_docs - 1,
        s.count(Arc::new(TermQuery::new(Term::from_text(
            "field1", "standard",
        ))))?,
    );
    Assert::assert_equals(
        num_docs - 1,
        s.count(Arc::new(TermQuery::new(Term::from_text(
            "field2", "memory",
        ))))?,
    );
    r.close()?;

    if *VERBOSE {
        println!("\nTEST: now delete 2nd doc");
    }
    w.delete_documents_by_terms(vec![Term::from_text("id", "44")])?;

    if *VERBOSE {
        println!("\nTEST: now force merge");
    }
    w.force_merge(1)?;
    if *VERBOSE {
        println!("\nTEST: now open reader");
    }
    let r = directory_reader::open_with_writer(&*w)?;
    Assert::assert_equals(num_docs - 2, r.max_doc());
    Assert::assert_equals(num_docs - 2, r.num_docs());
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    Assert::assert_equals(
        num_docs - 2,
        s.count(Arc::new(TermQuery::new(Term::from_text(
            "field1", "standard",
        ))))?,
    );
    Assert::assert_equals(
        num_docs - 2,
        s.count(Arc::new(TermQuery::new(Term::from_text(
            "field2", "memory",
        ))))?,
    );
    Assert::assert_equals(
        1,
        s.count(Arc::new(TermQuery::new(Term::from_text("id", "76"))))?,
    );
    Assert::assert_equals(
        0,
        s.count(Arc::new(TermQuery::new(Term::from_text("id", "77"))))?,
    );
    Assert::assert_equals(
        0,
        s.count(Arc::new(TermQuery::new(Term::from_text("id", "44"))))?,
    );

    if *VERBOSE {
        println!("\nTEST: now close NRT reader");
    }
    r.close()?;

    w.close()?;

    dir.close()
}

fn external_codecs_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestExternalCodecs")
        .method(t("testPerFieldCodec", external_codecs_test_per_field_codec))
        .build()
}

// -----------------------------------------------------------------------------
// TestMergeSchedulerExternal
// -----------------------------------------------------------------------------

/// `infoStream` (`volatile InfoStream`), cleared by `after()`.
static INFO_STREAM: Mutex<Option<Arc<dyn rucene::util::InfoStream>>> = Mutex::new(None);

/// `mergeCalled`, `mergeThreadCreated`, `excCalled` (`volatile boolean`).
static MERGE_CALLED: AtomicBool = AtomicBool::new(false);
static MERGE_THREAD_CREATED: AtomicBool = AtomicBool::new(false);
static EXC_CALLED: AtomicBool = AtomicBool::new(false);

/// `infoStream`, as the Java code reads it.
fn info_stream() -> Option<Arc<dyn rucene::util::InfoStream>> {
    INFO_STREAM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// `MyMergeScheduler` (`private class … extends ConcurrentMergeScheduler`),
/// with its `MyMergeThread` (`private class … extends
/// ConcurrentMergeScheduler.MergeThread`).
///
/// **Divergence from Lucene 10.5.0.** Java subclasses the scheduler and
/// overrides its protected `getMergeThread`, `handleMergeException` and
/// `doMerge`; Rucene's `ConcurrentMergeScheduler` takes these overrides as
/// hooks.
fn my_merge_scheduler() -> Arc<ConcurrentMergeScheduler> {
    let cms = ConcurrentMergeScheduler::new();
    cms.set_get_merge_thread_override(Some(Arc::new(|_super, merge_source, merge| {
        // `new MyMergeThread(mergeSource, merge)`, then `setDaemon(true)`
        // and `setName("MyMergeThread")`.
        let thread = MergeThread::new(merge_source, merge, "MyMergeThread".to_string());
        MERGE_THREAD_CREATED.store(true, Ordering::SeqCst);
        Ok(Arc::new(thread))
    })));
    cms.set_handle_merge_exception_override(Some(Arc::new(|_t| {
        EXC_CALLED.store(true, Ordering::SeqCst);
        if let Some(info_stream) = info_stream() {
            if info_stream.is_enabled("IW") {
                info_stream.message("IW", "TEST: now handleMergeException");
            }
        }
    })));
    cms.set_do_merge_override(Some(Arc::new(|cms, merge_source, merge| {
        MERGE_CALLED.store(true, Ordering::SeqCst);
        cms.default_do_merge(merge_source, merge)
    })));
    Arc::new(cms)
}

/// `FailOnlyOnMerge` (`private class … extends MockDirectoryWrapper.Failure`).
///
/// `callStackContainsAnyOf("doMerge")` names the Rust frame of
/// `ConcurrentMergeScheduler::do_merge`, `do_merge`.
struct FailOnlyOnMerge {
    do_fail: AtomicBool,
}

impl Failure for FailOnlyOnMerge {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if LuceneTestCase::call_stack_contains_any_of(&["do_merge"]) {
            let ioe = LuceneError::from(LuceneErrorKind::Io(std::io::Error::other(
                "now failing during merge",
            )));
            // `ioe.printStackTrace(pw)`: the throwable's `toString()` line;
            // Rust records no Java stack frames to follow it.
            let sw = "java.io.IOException: now failing during merge\n";
            if let Some(info_stream) = info_stream() {
                if info_stream.is_enabled("IW") {
                    info_stream.message("IW", &format!("TEST: now throw exc:\n{sw}"));
                }
            }
            return Err(ioe);
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// A `ByteArrayOutputStream` a `PrintStream` writes to.
#[derive(Clone, Default)]
struct ByteArrayOutputStream(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for ByteArrayOutputStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `TestMergeSchedulerExternal.testSubclassConcurrentMergeScheduler()`.
fn merge_scheduler_external_test_subclass_concurrent_merge_scheduler() -> Result<()> {
    let mut dir = LuceneTestCase::new_mock_directory();
    dir.fail_on(Arc::new(FailOnlyOnMerge {
        do_fail: AtomicBool::new(false),
    }));

    let mut doc = Document::new();
    let id_field = LuceneTestCase::new_string_field("id", "", Store::YES);
    doc.add(Box::new(id_field));

    let merge_scheduler = my_merge_scheduler();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_merge_scheduler(Some(Arc::clone(&merge_scheduler) as Arc<dyn MergeScheduler>))?;
    iwc.set_max_buffered_docs(2)?;
    iwc.set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;

    let baos = ByteArrayOutputStream::default();
    let info_stream: Arc<dyn rucene::util::InfoStream> =
        Arc::new(PrintStreamInfoStream::new(Box::new(baos.clone())));
    *INFO_STREAM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&info_stream));
    iwc.set_info_stream(info_stream)?;

    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let log_mp = writer.get_config().merge_policy();
    log_mp
        .as_any()
        .and_then(|any| any.downcast_ref::<LogMergePolicy>())
        .expect("(LogMergePolicy) writer.getConfig().getMergePolicy()")
        .set_merge_factor(10)?;

    let added = (|| -> Result<()> {
        for _ in 0..60 {
            writer.add_document(&doc)?;
        }
        Ok(())
    })();
    if let Err(e) = added {
        if !is_instance(&e, "java.lang.IllegalStateException") {
            return Err(e);
        }
        // OK
    }

    merge_scheduler.sync();
    writer.rollback()?;

    let created = MERGE_THREAD_CREATED.load(Ordering::SeqCst);
    let called = MERGE_CALLED.load(Ordering::SeqCst);
    let exc = EXC_CALLED.load(Ordering::SeqCst);
    if !(created && called && exc) {
        // `catch (AssertionError ae)`: print the IW infoStream output, rethrow.
        println!("TEST FAILED; IW infoStream output:");
        println!(
            "{}",
            String::from_utf8_lossy(&baos.0.lock().unwrap_or_else(|e| e.into_inner()))
        );
    }
    Assert::assert_true(created);
    Assert::assert_true(called);
    Assert::assert_true(exc);
    dir.close()
}

/// `ReportingMergeScheduler` (`private static class … extends
/// MergeScheduler`).
struct ReportingMergeScheduler {
    executor: Arc<SameThreadExecutorService>,
}

impl std::fmt::Display for ReportingMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReportingMergeScheduler")
    }
}

impl MergeScheduler for ReportingMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        while let Some(merge) = merge_source.get_next_merge()? {
            if *VERBOSE {
                println!("executing merge {}", merge.seg_string());
            }
            merge_source.merge(merge)?;
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        &self.executor
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// `TestMergeSchedulerExternal.testCustomMergeScheduler()`: "we don't really
/// need to execute anything, just to make sure the custom MS compiles. But
/// ensure that it can be used as well, e.g., no other hidden dependencies or
/// something. Therefore, don't use any random API !"
fn merge_scheduler_external_test_custom_merge_scheduler() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(rucene::store::ByteBuffersDirectory::new());
    let conf = IndexWriterConfig::with_analyzer(None);
    conf.set_merge_scheduler(Some(Arc::new(ReportingMergeScheduler {
        executor: Arc::new(SameThreadExecutorService::new()),
    })))?;
    let writer = IndexWriter::new(Arc::clone(&dir), conf)?;
    writer.add_document(&Document::new())?;
    writer.commit()?; // trigger flush
    writer.add_document(&Document::new())?;
    writer.commit()?; // trigger flush
    writer.force_merge(1)?;
    writer.close()?;
    close_arc_directory(dir)
}

fn merge_scheduler_external_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestMergeSchedulerExternal")
        .method(Method::new("after").annotate(After).body_any(|| {
            *INFO_STREAM.lock().unwrap_or_else(|e| e.into_inner()) = None;
            MERGE_CALLED.store(false, Ordering::SeqCst);
            MERGE_THREAD_CREATED.store(false, Ordering::SeqCst);
            EXC_CALLED.store(false, Ordering::SeqCst);
        }))
        .method(t(
            "testSubclassConcurrentMergeScheduler",
            merge_scheduler_external_test_subclass_concurrent_merge_scheduler,
        ))
        .method(t(
            "testCustomMergeScheduler",
            merge_scheduler_external_test_custom_merge_scheduler,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestSearchForDuplicates
// -----------------------------------------------------------------------------

const PRIORITY_FIELD: &str = "priority";
const ID_FIELD: &str = "id";
const HIGH_PRIORITY: &str = "high";
const MED_PRIORITY: &str = "medium";
#[allow(dead_code)]
const LOW_PRIORITY: &str = "low";

/// `TestSearchForDuplicates.testRun()`: compares search results when using
/// and not using compound files.
fn search_for_duplicates_test_run() -> Result<()> {
    let mut sw = String::new();
    let max_docs = LuceneTestCase::at_least(225);
    do_test_duplicates(&*random(), &mut sw, false, max_docs)?;
    let multi_file_output = sw;
    // System.out.println(multiFileOutput);

    let mut sw = String::new();
    do_test_duplicates(&*random(), &mut sw, true, max_docs)?;
    let single_file_output = sw;

    Assert::assert_equals_str(None, Some(&multi_file_output), Some(&single_file_output));
    Ok(())
}

/// `doTest(Random, PrintWriter, boolean, int)` (`private`).
fn do_test_duplicates(
    random: &dyn Random,
    out: &mut String,
    use_compound_files: bool,
    max_docs: i32,
) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(random));
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    let mp = conf.merge_policy();
    mp.set_no_cfs_ratio(if use_compound_files { 1.0 } else { 0.0 })?;
    let writer = IndexWriter::new(directory.as_directory(), conf)?;
    if *VERBOSE {
        println!("TEST: now build index MAX_DOCS={max_docs}");
    }

    for j in 0..max_docs {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            PRIORITY_FIELD,
            HIGH_PRIORITY,
            Store::YES,
        )));
        d.add(Box::new(StoredField::new_number(
            ID_FIELD,
            rucene::document::NumericValue::Int(j),
        )?));
        d.add(Box::new(NumericDocValuesField::new(ID_FIELD, j as i64)));
        writer.add_document(&d)?;
    }
    writer.close()?;

    // try a search without OR
    let reader = directory_reader::open(directory.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(
        PRIORITY_FIELD,
        HIGH_PRIORITY,
    )));
    let _ = writeln!(out, "Query: {}", query.to_query_string(PRIORITY_FIELD));
    if *VERBOSE {
        println!(
            "TEST: search query={}",
            rucene::search::query::query_to_string(&*query)
        );
    }

    let sort = Sort::new_fields(vec![
        SortField::FIELD_SCORE,
        SortField::new(Some(ID_FIELD.to_string()), SortFieldType::Int)?,
    ])?;

    let hits = searcher
        .search_with_sort(Arc::clone(&query), max_docs, &sort)?
        .score_docs;
    print_hits_duplicates(out, &hits, &searcher)?;
    check_hits(&hits, max_docs, &searcher)?;

    // try a new search with OR
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let mut boolean_query = rucene::search::boolean_query::Builder::new();
    boolean_query.add(
        Arc::new(TermQuery::new(Term::from_text(
            PRIORITY_FIELD,
            HIGH_PRIORITY,
        ))),
        Occur::SHOULD,
    )?;
    boolean_query.add(
        Arc::new(TermQuery::new(Term::from_text(
            PRIORITY_FIELD,
            MED_PRIORITY,
        ))),
        Occur::SHOULD,
    )?;
    let _ = writeln!(
        out,
        "Query: {}",
        boolean_query.build().to_query_string(PRIORITY_FIELD)
    );

    let hits = searcher
        .search_with_sort(Arc::new(boolean_query.build()), max_docs, &sort)?
        .score_docs;
    print_hits_duplicates(out, &hits, &searcher)?;
    check_hits(&hits, max_docs, &searcher)?;

    reader.close()?;
    directory.close()
}

/// `printHits(PrintWriter, ScoreDoc[], IndexSearcher)` (`private`).
fn print_hits_duplicates(
    out: &mut String,
    hits: &[FieldDoc],
    searcher: &rucene::search::index_searcher::IndexSearcher,
) -> Result<()> {
    let _ = writeln!(out, "{} total results\n", hits.len());
    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in hits.iter().enumerate() {
        if i < 10 || (i > 94 && i < 105) {
            let d = stored_fields.document(hit.doc())?;
            let _ = writeln!(
                out,
                "{i} {}",
                d.get(ID_FIELD).unwrap_or_else(|| "null".into())
            );
        }
    }
    Ok(())
}

/// `checkHits(ScoreDoc[], int, IndexSearcher)` (`private`).
fn check_hits(
    hits: &[FieldDoc],
    expected_count: i32,
    searcher: &rucene::search::index_searcher::IndexSearcher,
) -> Result<()> {
    Assert::assert_equals_msg(Some("total results"), expected_count, hits.len() as i32);
    let stored_fields = searcher.stored_fields()?;
    for (i, hit) in hits.iter().enumerate() {
        if i < 10 || (i > 94 && i < 105) {
            let d = stored_fields.document(hit.doc())?;
            Assert::assert_equals_str(
                Some(&format!("check {i}")),
                Some(&i.to_string()),
                d.get(ID_FIELD).as_deref(),
            );
        }
    }
    Ok(())
}

fn search_for_duplicates_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestSearchForDuplicates")
        .method(t("testRun", search_for_duplicates_test_run))
        .build()
}

// -----------------------------------------------------------------------------
// TestSearch
// -----------------------------------------------------------------------------

/// `TestSearch.testSearch()`: performs a number of searches and compares the
/// output of searches using multi-file index segments with single-file index
/// segments.
fn search_test_search() -> Result<()> {
    let mut sw = String::new();
    do_test_search(&*random(), &mut sw, false)?;
    let multi_file_output = sw;
    // System.out.println(multiFileOutput);

    let mut sw = String::new();
    do_test_search(&*random(), &mut sw, true)?;
    let single_file_output = sw;

    Assert::assert_equals_str(None, Some(&multi_file_output), Some(&single_file_output));
    Ok(())
}

/// `doTestSearch(Random, PrintWriter, boolean)` (`private`).
fn do_test_search(random: &dyn Random, out: &mut String, use_compound_file: bool) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(random));
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    let mp = conf.merge_policy();
    mp.set_no_cfs_ratio(if use_compound_file { 1.0 } else { 0.0 })?;
    let writer = IndexWriter::new(directory.as_directory(), conf)?;

    let docs = [
        "a b c d e",
        "a b c d e a b c d e",
        "a b c d e f g h i j",
        "a c e",
        "e c a",
        "a c e a c e",
        "a c e a b c",
    ];
    for (j, text) in docs.iter().enumerate() {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            "contents",
            text,
            Store::YES,
        )));
        d.add(Box::new(NumericDocValuesField::new("id", j as i64)));
        writer.add_document(&d)?;
    }
    writer.close()?;

    let reader = directory_reader::open(directory.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let sort = Sort::new_fields(vec![
        SortField::FIELD_SCORE,
        SortField::new(Some("id".to_string()), SortFieldType::Int)?,
    ])?;

    for query in build_queries()? {
        let _ = writeln!(out, "Query: {}", query.to_query_string("contents"));
        if *VERBOSE {
            println!(
                "TEST: query={}",
                rucene::search::query::query_to_string(&*query)
            );
        }

        let hits = searcher.search_with_sort(query, 1000, &sort)?.score_docs;

        let _ = writeln!(out, "{} total results", hits.len());
        let stored_fields = searcher.stored_fields()?;
        for (i, hit) in hits.iter().enumerate().take(10) {
            let d = stored_fields.document(hit.doc())?;
            let _ = writeln!(
                out,
                "{i} {} {}",
                float_to_string(hit.score()),
                d.get("contents").unwrap_or_else(|| "null".into())
            );
        }
    }
    reader.close()?;
    directory.close()
}

/// `buildQueries()` (`private`).
fn build_queries() -> Result<Vec<Arc<dyn Query>>> {
    let mut queries: Vec<Arc<dyn Query>> = Vec::new();

    let mut boolean_ab = rucene::search::boolean_query::Builder::new();
    boolean_ab.add(
        Arc::new(TermQuery::new(Term::from_text("contents", "a"))),
        Occur::SHOULD,
    )?;
    boolean_ab.add(
        Arc::new(TermQuery::new(Term::from_text("contents", "b"))),
        Occur::SHOULD,
    )?;
    queries.push(Arc::new(boolean_ab.build()));

    let phrase_ab = PhraseQuery::from_strings("contents", &["a", "b"])?;
    queries.push(Arc::new(phrase_ab));

    let phrase_abc = PhraseQuery::from_strings("contents", &["a", "b", "c"])?;
    queries.push(Arc::new(phrase_abc));

    let mut boolean_ac = rucene::search::boolean_query::Builder::new();
    boolean_ac.add(
        Arc::new(TermQuery::new(Term::from_text("contents", "a"))),
        Occur::SHOULD,
    )?;
    boolean_ac.add(
        Arc::new(TermQuery::new(Term::from_text("contents", "c"))),
        Occur::SHOULD,
    )?;
    queries.push(Arc::new(boolean_ac.build()));

    let phrase_ac = PhraseQuery::from_strings("contents", &["a", "c"])?;
    queries.push(Arc::new(phrase_ac));

    let phrase_ace = PhraseQuery::from_strings("contents", &["a", "c", "e"])?;
    queries.push(Arc::new(phrase_ace));

    Ok(queries)
}

fn search_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.TestSearch")
        .method(t("testSearch", search_test_search))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_assertions() {
    run(assertions_class);
}

#[test]
fn test_demo() {
    run(demo_class);
}

#[test]
fn test_external_codecs() {
    run(external_codecs_class);
}

#[test]
fn test_merge_scheduler_external() {
    run(merge_scheduler_external_class);
}

#[test]
fn test_search_for_duplicates() {
    run(search_for_duplicates_class);
}

#[test]
fn test_search() {
    run(search_class);
}
