//! Port of `lucene/core/src/test/org/apache/lucene/index/TestThreadedForceMerge.java`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, FieldType, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{English, LuceneTestCase};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::{npe, static_hook};

/// `TestThreadedForceMerge`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestThreadedForceMerge",
        vec![
            static_hook("setup", BeforeClass, setup),
            test("testThreadedForceMerge", test_threaded_force_merge),
        ],
    )
}

/// `private static Analyzer ANALYZER`.
static ANALYZER: Mutex<Option<Arc<dyn Analyzer>>> = Mutex::new(None);

const NUM_THREADS: i32 = 3;
// private final static int NUM_THREADS = 5;

const NUM_ITER: i32 = 1;

const NUM_ITER2: i32 = 1;

/// `ANALYZER`.
fn analyzer() -> Arc<dyn Analyzer> {
    ANALYZER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| npe("ANALYZER"))
}

/// `@BeforeClass public static void setup()`.
fn setup() -> Result<()> {
    *ANALYZER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&SIMPLE), true),
    ));
    Ok(())
}

/// `(LogMergePolicy) writer.getConfig().getMergePolicy()`, `setMergeFactor(merge_factor)`.
fn set_merge_factor(writer: &IndexWriter, merge_factor: i32) -> Result<()> {
    let mp = writer.get_config().merge_policy();
    mp.as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a LogMergePolicy",
            )
            .throw()
        })
        .set_merge_factor(merge_factor)?;
    Ok(())
}

/// The body of one thread of `runTest`.
fn thread_body(
    writer_final: &IndexWriter,
    custom_type: &FieldType,
    iter_final: i32,
    i_final: i32,
) -> Result<()> {
    for j in 0..NUM_ITER2 {
        writer_final.force_merge_with(1, false)?;
        for k in 0..17 * (1 + i_final) {
            let mut d = Document::new();
            d.add(Box::new(LuceneTestCase::new_field(
                "id",
                &format!("{iter_final}_{i_final}_{j}_{k}"),
                custom_type,
            )));
            d.add(Box::new(LuceneTestCase::new_field(
                "contents",
                &English::int_to_english(i_final + k),
                custom_type,
            )));
            writer_final.add_document(&d)?;
        }
        for k in 0..9 * (1 + i_final) {
            writer_final.delete_documents_by_terms(vec![Term::from_text(
                "id",
                &format!("{iter_final}_{i_final}_{j}_{k}"),
            )])?;
        }
        writer_final.force_merge(1)?;
    }
    Ok(())
}

/// `public void runTest(Random random, Directory directory)`.
fn run_test(_random: &dyn Random, directory: &Arc<dyn Directory>) -> Result<()> {
    let failed = Arc::new(AtomicBool::new(false));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer()));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let mut writer = IndexWriter::new(Arc::clone(directory), iwc)?;

    for iter in 0..NUM_ITER {
        let iter_final = iter;

        set_merge_factor(&writer, 1000)?;

        let custom_type = FieldType::new_from(StringField::type_stored());
        custom_type.set_omit_norms(true)?;
        let custom_type = Arc::new(custom_type);

        for i in 0..200 {
            let mut d = Document::new();
            d.add(Box::new(LuceneTestCase::new_field(
                "id",
                &i.to_string(),
                &custom_type,
            )));
            d.add(Box::new(LuceneTestCase::new_field(
                "contents",
                &English::int_to_english(i),
                &custom_type,
            )));
            writer.add_document(&d)?;
        }

        set_merge_factor(&writer, 4)?;

        let mut threads: Vec<Thread> = Vec::new();

        for i in 0..NUM_THREADS {
            let i_final = i;
            let writer_final = Arc::clone(&writer);
            let custom_type = Arc::clone(&custom_type);
            let failed = Arc::clone(&failed);
            threads.push(Thread::unnamed(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    thread_body(&writer_final, &custom_type, iter_final, i_final)
                }));
                let t = match outcome {
                    Ok(Ok(())) => return,
                    Ok(Err(e)) => java_throwable(&e),
                    Err(payload) => {
                        randomizedtesting::jdk::exception::Throwable::from_panic(payload)
                    }
                };
                // setFailed()
                failed.store(true, Ordering::SeqCst);
                println!("{}: hit exception", Thread::current().name());
                println!("{t}");
            }));
        }

        for thread in &threads {
            thread.start();
        }

        for thread in &threads {
            let _ = thread.join();
        }

        Assert::assert_true(!failed.load(Ordering::SeqCst));

        let expected_doc_count = ((1 + iter) as f64
            * (200.0
                + 8.0
                    * f64::from(NUM_ITER2)
                    * (f64::from(NUM_THREADS) / 2.0)
                    * f64::from(1 + NUM_THREADS))) as i32;

        let message = || -> Result<String> {
            Ok(format!(
                "index={} numDocs={} maxDoc={} config={}",
                writer.seg_string()?,
                writer.get_doc_stats()?.num_docs,
                writer.get_doc_stats()?.max_doc,
                writer.get_config()
            ))
        };
        assert_equals_msg(
            Some(&message()?),
            &expected_doc_count,
            &writer.get_doc_stats()?.num_docs,
        );
        assert_equals_msg(
            Some(&message()?),
            &expected_doc_count,
            &writer.get_doc_stats()?.max_doc,
        );

        writer.close()?;
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer()));
        iwc.set_open_mode(OpenMode::APPEND)?;
        iwc.set_max_buffered_docs(2)?;
        writer = IndexWriter::new(Arc::clone(directory), iwc)?;

        let reader = directory_reader::open(Arc::clone(directory))?;
        assert_equals_msg(
            Some(&format!("reader={reader}")),
            &1,
            &(Arc::clone(&reader) as Arc<dyn IndexReader>)
                .leaves()?
                .len(),
        );
        assert_equals(&expected_doc_count, &reader.num_docs());
        reader.close()?;
    }
    writer.close()
}

/// Port of `TestThreadedForceMerge.testThreadedForceMerge`.
fn test_threaded_force_merge() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    run_test(&*random(), &directory.as_directory())?;
    directory.close()
}
