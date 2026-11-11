//! Port of `lucene/core/src/test/org/apache/lucene/index/TestRollback.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::terms::Term;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, random, test};

/// `TestRollback`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestRollback",
        vec![test(
            "testRollbackIntegrityWithBufferFlush",
            test_rollback_integrity_with_buffer_flush,
        )],
    )
}

/// LUCENE-2536.
///
/// Port of `TestRollback.testRollbackIntegrityWithBufferFlush`.
fn test_rollback_integrity_with_buffer_flush() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let rw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for i in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "pk",
            &i.to_string(),
            Store::YES,
        )));
        rw.add_document(&doc)?;
    }
    rw.close()?;

    // If buffer size is small enough to cause a flush, errors ensue...
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_max_buffered_docs(2)?;
    conf.set_open_mode(OpenMode::APPEND)?;
    let w = IndexWriter::new(dir.as_directory(), conf)?;

    for i in 0..3 {
        let mut doc = Document::new();
        let value = i.to_string();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "pk",
            &value,
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "text",
            "foo",
            Store::YES,
        )));
        w.update_document(Term::from_text("pk", &value), &doc)?;
    }
    w.rollback()?;

    let r = directory_reader::open(dir.as_directory())?;
    assert_equals_msg(
        Some("index should contain same number of docs post rollback"),
        &5,
        &r.num_docs(),
    );
    r.close()?;
    dir.close()
}
