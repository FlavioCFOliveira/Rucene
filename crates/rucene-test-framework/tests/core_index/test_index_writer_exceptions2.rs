//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterExceptions2.java`.
//!
//! Causes a bunch of non-aborting and aborting exceptions and checks that
//! no index corruption is ever created
//!
//! * `catch (AlreadyClosedException)` / `catch (Exception e)` around a
//!   writer call catch the error the call returns; `Rethrow.rethrow(e)`
//!   returns it.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::io::Write as _;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
};
use rucene::codecs::Codec;
use rucene::document::{
    BinaryDocValuesField, Document, FieldType, IntPoint, NumericDocValuesField,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::{
    components, shared, sink, CrankyTokenFilter, MockVariableLengthPayloadFilter,
};
use rucene_test_framework::codecs::asserting::AssertingCodec;
use rucene_test_framework::codecs::cranky::CrankyCodec;
use rucene_test_framework::store::{MockDirectoryWrapper, Throttling};
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable};
use rucene_test_framework::util::lucene_test_case::{SuppressCodecs, RANDOM_MULTIPLIER};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test, verbose};

const ACE: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `TestIndexWriterExceptions2` (`@SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestIndexWriterExceptions2",
        vec![test("testBasics", test_basics)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// The anonymous `Analyzer` of `testBasics`: lots of non-aborting exceptions.
#[derive(Debug)]
struct BrokenAnalyzer {
    state: AnalyzerState,
    analyzer_seed: i64,
}

impl Analyzer for BrokenAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexWriterExceptions2$1"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&SIMPLE), false)?;
        tokenizer.set_enable_checks(false); // TODO: can we turn this on? our filter is probably too evil
        let tokenizer = Arc::new(AtomicRefCell::new(tokenizer));
        let mut stream: Box<dyn TokenStream> = shared(&tokenizer);
        // emit some payloads
        if field_name.contains("payloads") {
            stream = Box::new(MockVariableLengthPayloadFilter::new(
                Arc::new(JdkRandom::new(self.analyzer_seed)),
                stream,
            )?);
        }
        let stream = CrankyTokenFilter::new(stream, Arc::new(JdkRandom::new(self.analyzer_seed)));
        Ok(components(tokenizer, sink(stream)))
    }
}

/// `exceptionLog`: the `ByteArrayOutputStream` the `PrintStream` writes to.
#[derive(Clone, Default)]
struct ExceptionLog(Arc<Mutex<Vec<u8>>>);

impl ExceptionLog {
    /// `exceptionStream.println(...)` followed by `e.printStackTrace(exceptionStream)`.
    fn log(&self, e: &LuceneError) {
        let mut log = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let t = java_throwable(e);
        let _ = writeln!(
            log,
            "\nTEST: got expected fake exc:{}",
            t.message().unwrap_or("null")
        );
        let _ = writeln!(log, "{t}");
    }

    /// `exceptionLog.toString(UTF_8)`.
    fn to_string_utf8(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|p| p.into_inner())).into_owned()
    }
}

/// `e.getMessage() != null && e.getMessage().startsWith("Fake IOException")`.
fn is_fake(e: &LuceneError) -> bool {
    java_throwable(e)
        .message()
        .is_some_and(|m| m.starts_with("Fake IOException"))
}

/// `newIndexWriterConfig(analyzer)` with a `SerialMergeScheduler` (just for
/// now, try to keep this test reproducible) and `codec`.
fn config(analyzer: &Arc<dyn Analyzer>, codec: &Arc<dyn Codec>) -> Result<IndexWriterConfig> {
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(analyzer)));
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    conf.set_codec(Arc::clone(codec))?;
    Ok(conf)
}

/// The two `catch` clauses every guarded block of `testBasics` shares: an
/// `AlreadyClosedException` reopens the writer, a fake exception is logged
/// and allows the next one, anything else is rethrown.
fn handle(
    result: Result<()>,
    iw: &mut Arc<IndexWriter>,
    allow_already_closed: &mut bool,
    dir: &Arc<dyn rucene::store::Directory>,
    analyzer: &Arc<dyn Analyzer>,
    codec: &Arc<dyn Codec>,
    log: &ExceptionLog,
) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(ace) if is_instance(&ace, ACE) => {
            // OK: writer was closed by abort; we just reopen now:
            Assert::assert_true(iw.is_deleter_closed()?);
            Assert::assert_true(*allow_already_closed);
            *allow_already_closed = false;
            let conf = config(analyzer, codec)?;
            *iw = IndexWriter::new(Arc::clone(dir), conf)?;
            Ok(())
        }
        Err(e) => {
            if is_fake(&e) {
                log.log(&e);
                *allow_already_closed = true;
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

/// `new BytesRef(Integer.toString(i))`.
fn br(i: i32) -> BytesRef {
    BytesRef::from_char_sequence(&i.to_string())
}

/// just one thread, serial merge policy, hopefully debuggable
/// Port of `TestIndexWriterExceptions2.testBasics`.
fn test_basics() -> Result<()> {
    // disable slow things: we don't rely upon sleeps here.
    let mut dir_owner = LuceneTestCase::new_directory();
    if let Some(mock) = dir_owner.as_mock() {
        let mock: &MockDirectoryWrapper = mock;
        mock.set_throttling(Throttling::Never);
        mock.set_use_slow_open_closers(false);
    }
    let dir = dir_owner.as_directory();

    // log all exceptions we hit, in case we fail (for debugging)
    let exception_log = ExceptionLog::default();

    // create lots of non-aborting exceptions with a broken analyzer
    let analyzer_seed = random().next_long();
    let analyzer: Arc<dyn Analyzer> = Arc::new(BrokenAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        analyzer_seed,
    });

    // create lots of aborting exceptions with a broken codec
    // we don't need a random codec, as we aren't trying to find bugs in the codec here.
    let inner: Arc<dyn Codec> = if *RANDOM_MULTIPLIER > 1 {
        rucene::codecs::get_default().unwrap_or_else(|| crate::support::npe("codec"))
    } else {
        Arc::new(AssertingCodec::new())
    };
    let codec: Arc<dyn Codec> = Arc::new(CrankyCodec::new(
        inner,
        Arc::new(JdkRandom::new(random().next_long())) as Arc<dyn Random>,
    ));

    let conf = config(&analyzer, &codec)?;

    let num_docs = LuceneTestCase::at_least(100);

    let mut iw = IndexWriter::new(Arc::clone(&dir), conf)?;
    let result = (|| -> Result<()> {
        let mut allow_already_closed = false;
        for i in 0..num_docs {
            // TODO: add crankyDocValuesFields, etc
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
                        iw.delete_documents_by_terms(vec![Term::from_text("id", &i.to_string())])?;
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
                handle(
                    r,
                    &mut iw,
                    &mut allow_already_closed,
                    &dir,
                    &analyzer,
                    &codec,
                    &exception_log,
                )?;
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
                handle(
                    r,
                    &mut iw,
                    &mut allow_already_closed,
                    &dir,
                    &analyzer,
                    &codec,
                    &exception_log,
                )?;
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
                    if directory_reader::index_exists(&*dir)? {
                        TestUtil::check_index(Arc::clone(&dir))?;
                    }
                    Ok(())
                })();
                handle(
                    r,
                    &mut iw,
                    &mut allow_already_closed,
                    &dir,
                    &analyzer,
                    &codec,
                    &exception_log,
                )?;
            }
        }

        if let Err(e) = iw.close() {
            if is_fake(&e) {
                exception_log.log(&e);
                let _ = iw.rollback();
            } else {
                return Err(e);
            }
        }
        drop(dir);
        dir_owner.close()
    })();
    if let Err(t) = result {
        println!("Unexpected exception: dumping fake-exception-log:...");
        println!("{}", exception_log.to_string_utf8());
        return Err(t);
    }

    if verbose() {
        println!("TEST PASSED: dumping fake-exception-log:...");
        println!("{}", exception_log.to_string_utf8());
    }
    Ok(())
}
