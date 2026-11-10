//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterOnError.java`.
//!
//! Causes a bunch of fake VM errors and checks that no other exceptions are delivered instead, no
//! index corruption is ever created.
//!
//! * A fake `Error` thrown by a `Failure` is the
//!   [`LuceneError::Unchecked`] of its Java class, returned from `eval`;
//!   `catch (Error | AlreadyClosedException disaster)` catches the error a
//!   writer call returns. `Rethrow.rethrow(t)` returns the error.
//! * `continue STARTOVER` returns from the iteration's body.

use std::io::Write as _;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::document::{
    BinaryDocValuesField, Document, FieldType, IntPoint, NumericDocValuesField,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_file_deleter::IndexFileDeleter;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, MockVariableLengthPayloadFilter,
};
use rucene_test_framework::store::{Failure, MockDirectoryWrapper, Throttling};
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{Nightly, SuppressCodecs, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test, verbose};

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";
const ERROR: &str = "java.lang.Error";

/// `TestIndexWriterOnError` (`@SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterOnError",
        vec![
            test("testOOM", test_oom),
            test("testUnknownError", test_unknown_error),
            test("testLinkageError", test_linkage_error),
            test("testIOError", test_io_error),
            test("testCheckpoint", test_checkpoint).annotate(Nightly),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// The anonymous `Analyzer` of `doTest`.
#[derive(Debug)]
struct AngryAnalyzer {
    state: AnalyzerState,
    analyzer_seed: i64,
}

impl Analyzer for AngryAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterOnError$1"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        tokenizer.set_enable_checks(false); // we are going to make it angry
        let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
        // emit some payloads
        if field_name.contains("payloads") {
            let stream: Box<dyn TokenStream> = shared(&tokenizer);
            let stream = MockVariableLengthPayloadFilter::new(
                Arc::new(JdkRandom::new(self.analyzer_seed)),
                stream,
            )?;
            return Ok(components(tokenizer, sink(stream)));
        }
        Ok(tokenizer_components(tokenizer))
    }
}

/// `exceptionLog`: the `ByteArrayOutputStream` the `PrintStream` writes to.
#[derive(Clone, Default)]
struct ExceptionLog(Arc<Mutex<Vec<u8>>>);

impl ExceptionLog {
    /// `exceptionStream.println(line)`.
    fn println(&self, line: &str) {
        let mut log = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let _ = writeln!(log, "{line}");
    }

    /// `exceptionLog.toString(UTF_8)`.
    fn to_string_utf8(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|p| p.into_inner())).into_owned()
    }
}

/// `new BytesRef(Integer.toString(i))`.
fn br(i: i32) -> BytesRef {
    BytesRef::from_char_sequence(&i.to_string())
}

/// just one thread, serial merge policy, hopefully debuggable
///
/// The private `doTest(MockDirectoryWrapper.Failure)`.
fn do_test(fail_on: Arc<dyn Failure>) -> Result<()> {
    // log all exceptions we hit, in case we fail (for debugging)
    let exception_log = ExceptionLog::default();
    // PrintStream exceptionStream = System.out;

    let analyzer_seed = random().next_long();
    let analyzer: Arc<dyn Analyzer> = Arc::new(AngryAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        analyzer_seed,
    });

    let mut dir: Option<MockDirectoryWrapper> = None;

    let num_iterations = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(100)
    } else {
        LuceneTestCase::at_least(5)
    };

    // STARTOVER:
    for _iter in 0..num_iterations {
        let result = (|| -> Result<()> {
            // close from last run
            if let Some(mut d) = dir.take() {
                d.close()?;
            }
            // disable slow things: we don't rely upon sleeps here.
            let d = dir.insert(LuceneTestCase::new_mock_directory());
            d.set_throttling(Throttling::Never);
            d.set_use_slow_open_closers(false);

            let conf =
                LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
            // just for now, try to keep this test reproducible
            conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;

            // test never makes it this far...
            let num_docs = LuceneTestCase::at_least(2000);

            let iw = IndexWriter::new(d.as_directory(), conf)?;
            iw.commit()?; // ensure there is always a commit

            d.fail_on(Arc::clone(&fail_on));

            for i in 0..num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_string_field(
                    "id",
                    &i.to_string(),
                    Store::NO,
                )));
                doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
                doc.add(Box::new(BinaryDocValuesField::new("dv2", br(i))));
                doc.add(Box::new(SortedDocValuesField::new("dv3", br(i))));
                doc.add(Box::new(SortedSetDocValuesField::new("dv4", br(i))));
                doc.add(Box::new(SortedSetDocValuesField::new("dv4", br(i - 1))));
                doc.add(Box::new(SortedNumericDocValuesField::new("dv5", i as i64)));
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "dv5",
                    (i - 1) as i64,
                )));
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "text1",
                    &TestUtil::random_analysis_string(&*random(), 20, true),
                    Store::NO,
                )));
                // ensure we store something
                doc.add(Box::new(StoredField::new_string(
                    "stored1",
                    "foo".to_string(),
                )?));
                doc.add(Box::new(StoredField::new_string(
                    "stored1",
                    "bar".to_string(),
                )?));
                // ensure we get some payloads
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "text_payloads",
                    &TestUtil::random_analysis_string(&*random(), 6, true),
                    Store::NO,
                )));
                // ensure we get some vectors
                let ft = FieldType::new_from(TextField::type_not_stored());
                ft.set_store_term_vectors(true)?;
                doc.add(Box::new(LuceneTestCase::new_field(
                    "text_vectors",
                    &TestUtil::random_analysis_string(&*random(), 6, true),
                    &ft,
                )));
                doc.add(Box::new(IntPoint::new("point", &[random().next_int()])?));
                let p0 = random().next_int();
                let p1 = random().next_int();
                doc.add(Box::new(IntPoint::new("point2d", &[p0, p1])?));

                if random().next_int_bound(10) > 0 {
                    // single doc
                    let r = (|| -> Result<()> {
                        iw.add_document(&doc)?;
                        // we made it, sometimes delete our doc, or update a dv
                        let thing_to_do = random().next_int_bound(4);
                        if thing_to_do == 0 {
                            iw.delete_documents_by_terms(vec![Term::from_text(
                                "id",
                                &i.to_string(),
                            )])?;
                        } else if thing_to_do == 1 {
                            iw.update_numeric_doc_value(
                                Term::from_text("id", &i.to_string()),
                                "dv",
                                i as i64 + 1,
                            )?;
                        } else if thing_to_do == 2 {
                            iw.update_binary_doc_value(
                                Term::from_text("id", &i.to_string()),
                                "dv2",
                                br(i + 1),
                            )?;
                        }
                        Ok(())
                    })();
                    if let Err(disaster) = r {
                        get_tragedy(disaster, &iw, &exception_log)?;
                        return Ok(()); // continue STARTOVER
                    }
                } else {
                    // block docs
                    let mut doc2 = Document::new();
                    doc2.add(Box::new(LuceneTestCase::new_string_field(
                        "id",
                        &(-i).to_string(),
                        Store::NO,
                    )));
                    doc2.add(Box::new(LuceneTestCase::new_text_field(
                        "text1",
                        &TestUtil::random_analysis_string(&*random(), 20, true),
                        Store::NO,
                    )));
                    doc2.add(Box::new(StoredField::new_string(
                        "stored1",
                        "foo".to_string(),
                    )?));
                    doc2.add(Box::new(StoredField::new_string(
                        "stored1",
                        "bar".to_string(),
                    )?));
                    doc2.add(Box::new(LuceneTestCase::new_field(
                        "text_vectors",
                        &TestUtil::random_analysis_string(&*random(), 6, true),
                        &ft,
                    )));

                    let docs = vec![doc, doc2];
                    let r = (|| -> Result<()> {
                        iw.add_documents(&docs)?;
                        // we made it, sometimes delete our docs
                        if random().next_boolean() {
                            iw.delete_documents_by_terms(vec![
                                Term::from_text("id", &i.to_string()),
                                Term::from_text("id", &(-i).to_string()),
                            ])?;
                        }
                        Ok(())
                    })();
                    if let Err(disaster) = r {
                        get_tragedy(disaster, &iw, &exception_log)?;
                        return Ok(()); // continue STARTOVER
                    }
                }

                if random().next_int_bound(10) == 0 {
                    // trigger flush:
                    let r = (|| -> Result<()> {
                        if random().next_boolean() {
                            let apply_all_deletes = random().next_boolean();
                            let ir = directory_reader::open_with_writer_flags(
                                &*iw,
                                apply_all_deletes,
                                false,
                            )?;
                            let checked =
                                TestUtil::check_reader(&(Arc::clone(&ir) as Arc<dyn IndexReader>));
                            let _ = ir.close();
                            checked?;
                        } else {
                            iw.commit()?;
                        }
                        if directory_reader::index_exists(&*d.as_directory())? {
                            TestUtil::check_index(d.as_directory())?;
                        }
                        Ok(())
                    })();
                    if let Err(disaster) = r {
                        get_tragedy(disaster, &iw, &exception_log)?;
                        return Ok(()); // continue STARTOVER
                    }
                }
            }

            if let Err(disaster) = iw.close() {
                get_tragedy(disaster, &iw, &exception_log)?;
                return Ok(()); // continue STARTOVER
            }
            Ok(())
        })();
        if let Err(t) = result {
            println!("Unexpected exception: dumping fake-exception-log:...");
            println!("{}", exception_log.to_string_utf8());
            return Err(t);
        }
    }
    if let Some(mut d) = dir.take() {
        d.close()?;
    }
    if verbose() {
        println!("TEST PASSED: dumping fake-exception-log:...");
        println!("{}", exception_log.to_string_utf8());
    }
    Ok(())
}

/// The private `getTragedy(disaster, writer, log)`: a fake `Error` (also as
/// the cause of an `AlreadyClosedException`) is logged and the writer rolled
/// back; anything else is rethrown.
fn get_tragedy(disaster: LuceneError, writer: &IndexWriter, log: &ExceptionLog) -> Result<()> {
    let thrown = java_throwable(&disaster);
    let e: Option<&Throwable> = if thrown.is_instance(ACE) {
        thrown.cause()
    } else {
        Some(&thrown)
    };

    match e {
        Some(e) if e.is_instance(ERROR) && e.message().is_some_and(|m| m.contains("Fake")) => {
            log.println(&format!(
                "\nTEST: got expected fake exc:{}",
                e.message().unwrap_or("null")
            ));
            log.println(&e.to_string());
            // TODO: remove rollback here, and add this assert to ensure "full OOM protection" anywhere IW
            // does writes
            // assertTrue("hit OOM but writer is still open, WTF: ", writer.isClosed());
            if let Err(t) = writer.rollback() {
                log.println(&t.to_string());
            }
            Ok(())
        }
        _ => Err(disaster),
    }
}

/// `new <class>(message)` for a fake Java `Error`.
fn fake_error(class: &'static str, message: &str) -> LuceneError {
    LuceneError::Unchecked {
        class,
        message: Some(message.to_string()),
        cause: None,
    }
}

/// What a [`RandomVmFailure`] throws.
#[derive(Clone, Copy)]
enum FakeError {
    OutOfMemory,
    Unknown,
    Linkage,
    Io,
}

impl FakeError {
    /// The error `eval` throws.
    fn error(self) -> LuceneError {
        match self {
            FakeError::OutOfMemory => {
                fake_error("java.lang.OutOfMemoryError", "Fake OutOfMemoryError")
            }
            FakeError::Unknown => fake_error("java.lang.UnknownError", "Fake UnknownError"),
            FakeError::Linkage => fake_error("java.lang.LinkageError", "Fake LinkageError"),
            // new IOError(new RuntimeException("Fake IOError")): the message is the
            // cause's toString().
            FakeError::Io => LuceneError::Unchecked {
                class: "java.io.IOError",
                message: Some("java.lang.RuntimeException: Fake IOError".to_string()),
                cause: Some(Arc::new(fake_error(
                    "java.lang.RuntimeException",
                    "Fake IOError",
                ))),
            },
        }
    }
}

/// The anonymous `Failure` of `testOOM`, `testUnknownError`,
/// `testLinkageError` and `testIOError`: one call in 3000 from inside
/// `IndexWriter` throws.
struct RandomVmFailure {
    do_fail: AtomicBool,
    r: JdkRandom,
    error: FakeError,
}

impl RandomVmFailure {
    fn new(error: FakeError) -> Arc<dyn Failure> {
        Arc::new(RandomVmFailure {
            do_fail: AtomicBool::new(false),
            r: JdkRandom::new(random().next_long()),
            error,
        })
    }
}

impl Failure for RandomVmFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.r.next_int_bound(3000) == 0 && LuceneTestCase::call_stack_contains::<IndexWriter>()
        {
            return Err(self.error.error());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterOnError.testOOM`.
fn test_oom() -> Result<()> {
    do_test(RandomVmFailure::new(FakeError::OutOfMemory))
}

/// Port of `TestIndexWriterOnError.testUnknownError`.
fn test_unknown_error() -> Result<()> {
    do_test(RandomVmFailure::new(FakeError::Unknown))
}

/// Port of `TestIndexWriterOnError.testLinkageError`.
fn test_linkage_error() -> Result<()> {
    do_test(RandomVmFailure::new(FakeError::Linkage))
}

/// Port of `TestIndexWriterOnError.testIOError`.
fn test_io_error() -> Result<()> {
    do_test(RandomVmFailure::new(FakeError::Io))
}

/// The anonymous `Failure` of `testCheckpoint`.
struct CheckpointFailure {
    do_fail: AtomicBool,
    r: JdkRandom,
}

impl Failure for CheckpointFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.r.next_int_bound(4) == 0
            && LuceneTestCase::call_stack_contains_method::<IndexFileDeleter>("checkpoint")
        {
            return Err(FakeError::OutOfMemory.error());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// Port of `TestIndexWriterOnError.testCheckpoint`.
fn test_checkpoint() -> Result<()> {
    let r = JdkRandom::new(random().next_long());
    do_test(Arc::new(CheckpointFailure {
        do_fail: AtomicBool::new(false),
        r,
    }))
}
