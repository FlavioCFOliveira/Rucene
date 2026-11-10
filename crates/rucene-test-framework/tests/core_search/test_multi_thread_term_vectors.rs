//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiThreadTermVectors.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{Fields, IndexReader, TermsEnum};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, random};

/// The instance fields of `TestMultiThreadTermVectors`.
#[derive(Default)]
struct Fixture {
    directory: Option<TestDirectory>,
    num_docs: i32,
    num_threads: i32,
    num_iterations: i32,
}

/// `TestMultiThreadTermVectors`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestMultiThreadTermVectors",
        vec![
            set_up(fixture_set_up),
            tear_down(|f: &mut Fixture| close_dir(f.directory.take())),
            ftest("test", test),
        ],
    )
}

fn fixture_set_up(f: &mut Fixture) -> Result<()> {
    f.num_docs = if *TEST_NIGHTLY { 1000 } else { 50 };
    f.num_threads = if *TEST_NIGHTLY { 3 } else { 2 };
    f.num_iterations = if *TEST_NIGHTLY { 100 } else { 50 };
    let directory = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(directory.as_directory(), iwc)?;
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_tokenized(false)?;
    custom_type.set_store_term_vectors(true)?;
    for i in 0..f.num_docs {
        let mut doc = Document::new();
        let fld = LuceneTestCase::new_field("field", &English::int_to_english(i), &custom_type);
        doc.add(Box::new(fld));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    f.directory = Some(directory);
    Ok(())
}

/// Port of `TestMultiThreadTermVectors.test`.
fn test(f: &mut Fixture) -> Result<()> {
    let reader = directory_reader::open(f.directory.as_ref().expect("setUp ran").as_directory())?;
    let outcome = test_term_position_vectors(
        Arc::clone(&reader) as Arc<dyn IndexReader>,
        f.num_threads,
        f.num_iterations,
    );
    reader.close()?;
    outcome
}

/// `testTermPositionVectors(reader, threadCount)`.
fn test_term_position_vectors(
    reader: Arc<dyn IndexReader>,
    thread_count: i32,
    num_iterations: i32,
) -> Result<()> {
    let mut mtr = Vec::with_capacity(thread_count as usize);
    for _ in 0..thread_count {
        mtr.push(multi_thread_term_vectors_reader(
            Arc::clone(&reader),
            num_iterations,
        ));
    }

    for vector_reader in &mtr {
        vector_reader.start();
    }
    for vector_reader in &mtr {
        let _ = vector_reader.join();
    }
    Ok(())
}

fn verify_vectors(vectors: &dyn Fields, num: i32) -> Result<()> {
    let fields: Vec<String> = vectors.iterator().collect();
    for field in fields {
        let terms = vectors.terms(&field)?;
        debug_assert!(terms.is_some());
        verify_vector(&mut *terms.expect("terms != null").iterator()?, num)?;
    }
    Ok(())
}

fn verify_vector(vector: &mut dyn TermsEnum, num: i32) -> Result<()> {
    let mut temp = String::new();
    while vector.next()?.is_some() {
        temp.push_str(&vector.term()?.utf8_to_string()?);
    }
    assert_equals(
        &English::int_to_english(num).trim().to_string(),
        &temp.trim().to_string(),
    );
    Ok(())
}

/// `class MultiThreadTermVectorsReader extends Thread`, initialised with
/// `reader`.
fn multi_thread_term_vectors_reader(reader: Arc<dyn IndexReader>, num_iterations: i32) -> Thread {
    Thread::unnamed(move || {
        let outcome: Result<()> = (|| {
            for _ in 0..num_iterations {
                test_term_vectors(&reader)?;
            }
            Ok(())
        })();
        if let Err(e) = outcome {
            Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw();
        }
    })
}

fn test_term_vectors(reader: &Arc<dyn IndexReader>) -> Result<()> {
    // check:
    let num_docs = reader.num_docs();
    let term_vectors = reader.term_vectors()?;
    for doc_id in 0..num_docs {
        let vectors = term_vectors.get(doc_id)?.expect("the doc has term vectors");
        // verify vectors result
        verify_vectors(&*vectors, doc_id)?;
        let vector = term_vectors
            .get(doc_id)?
            .expect("the doc has term vectors")
            .terms("field")?
            .expect("the field has a term vector");
        verify_vector(&mut *vector.iterator()?, doc_id)?;
    }
    Ok(())
}
