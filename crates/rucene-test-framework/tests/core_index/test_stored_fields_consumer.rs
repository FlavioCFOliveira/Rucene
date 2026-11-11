//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStoredFieldsConsumer.java`.
//!
//! The anonymous subclass is a `StoredFieldsConsumerOverrides`; the
//! `SegmentWriteState` the Java flush takes is passed as its segment, as
//! `StoredFieldsConsumer::flush` declares.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{SegmentInfo, StoredFieldsConsumer, StoredFieldsConsumerOverrides};
use rucene::search::Sort;
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestStoredFieldsConsumer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStoredFieldsConsumer",
        vec![test("testFinish", test_finish)],
    )
}

/// The anonymous `StoredFieldsConsumer` subclass that counts
/// `startDocument` and `finishDocument`.
struct CountingOverrides {
    start_doc_counter: Arc<AtomicI32>,
    finish_doc_counter: Arc<AtomicI32>,
}

impl StoredFieldsConsumerOverrides for CountingOverrides {
    fn start_document(&self, consumer: &mut StoredFieldsConsumer, doc_id: i32) -> Result<()> {
        consumer.super_start_document(doc_id)?;
        self.start_doc_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn finish_document(&self, consumer: &mut StoredFieldsConsumer) -> Result<()> {
        consumer.super_finish_document()?;
        self.finish_doc_counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Port of `TestStoredFieldsConsumer.testFinish`.
fn test_finish() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::new();
    let mut si = SegmentInfo::new(
        Arc::clone(&d),
        Version::LATEST,
        None,
        "_0".to_string(),
        -1,
        false,
        false,
        iwc.codec(),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )?;

    let start_doc_counter = Arc::new(AtomicI32::new(0));
    let finish_doc_counter = Arc::new(AtomicI32::new(0));
    let mut consumer = StoredFieldsConsumer::new(iwc.codec(), Arc::clone(&d), si.clone());
    consumer.set_overrides(Arc::new(CountingOverrides {
        start_doc_counter: Arc::clone(&start_doc_counter),
        finish_doc_counter: Arc::clone(&finish_doc_counter),
    }));

    let num_docs = 3;
    consumer.finish(num_docs)?;

    si.set_max_doc(num_docs)?;
    consumer.flush(&si)?;
    drop(d);
    dir.close()?;

    assert_equals(&num_docs, &start_doc_counter.load(Ordering::SeqCst));
    assert_equals(&num_docs, &finish_doc_counter.load(Ordering::SeqCst));
    Ok(())
}
