//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexManyDocuments.java`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `TestIndexManyDocuments`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexManyDocuments",
        vec![test("test", test_)],
    )
}

/// Port of `TestIndexManyDocuments.test`.
fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let iwc = IndexWriterConfig::new();
    iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 2000))?;
    let max_buffered_docs = iwc.max_buffered_docs();

    let num_docs = LuceneTestCase::at_least(10000);

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let count = Arc::new(AtomicI32::new(0));
    let mut threads = Vec::with_capacity(2);
    for _ in 0..2 {
        let w = Arc::clone(&w);
        let count = Arc::clone(&count);
        let thread = Thread::unnamed(move || {
            while count.fetch_add(1, Ordering::SeqCst) < num_docs {
                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_text_field(
                    "field",
                    "text",
                    Store::NO,
                )));
                if let Err(ioe) = w.add_document(&doc) {
                    rethrow_java(LuceneError::runtime_exception(ioe));
                }
            }
        });
        thread.start();
        threads.push(thread);
    }

    for thread in &threads {
        if let Err(payload) = thread.join() {
            std::panic::resume_unwind(payload);
        }
    }

    let max_doc = w.get_doc_stats()?.max_doc;
    assert_equals_msg(
        Some(&format!(
            "lost {} documents; maxBufferedDocs={max_buffered_docs}",
            num_docs - max_doc
        )),
        &num_docs,
        &max_doc,
    );
    w.close()?;

    let r = directory_reader::open(dir.as_directory())?;
    assert_equals(&num_docs, &r.max_doc());
    (r as Arc<dyn IndexReader>).close()?;
    dir.close()
}
