//! Port of `lucene/core/src/test/org/apache/lucene/index/TestInfoStream.java`.
//!
//! Tests indexwriter's infostream
//!
//! * `new IndexWriter(dir, iwc) { isEnableTestPoints() { return true; } }`
//!   is `IndexWriter::new_with_test_points`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::util::InfoStream;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestInfoStream`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestInfoStream",
        vec![
            test("testTestPointsOff", test_test_points_off),
            test("testTestPointsOn", test_test_points_on),
            test("testMergeSmallIndex", test_merge_small_index),
        ],
    )
}

/// The anonymous `InfoStream` of `testTestPointsOff`.
struct NoTestPoints;

impl InfoStream for NoTestPoints {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestInfoStream$1"
    }

    fn close(&self) {}

    fn message(&self, component: &str, _message: &str) {
        Assert::assert_false("TP" == component);
    }

    fn is_enabled(&self, component: &str) -> bool {
        Assert::assert_false("TP" == component);
        true
    }
}

/// we shouldn't have test points unless we ask
///
/// Port of `TestInfoStream.testTestPointsOff`.
fn test_test_points_off() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(None);
    iwc.set_info_stream(Arc::new(NoTestPoints))?;
    let iw = IndexWriter::new(dir.as_directory(), iwc)?;
    iw.add_document(&Document::new())?;
    iw.close()?;
    drop(iw);
    dir.close()
}

/// The anonymous `InfoStream` of `testTestPointsOn`.
struct SeenTestPoint(Arc<AtomicBool>);

impl InfoStream for SeenTestPoint {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestInfoStream$2"
    }

    fn close(&self) {}

    fn message(&self, component: &str, _message: &str) {
        if "TP" == component {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }
}

/// but they should work when we need
///
/// Port of `TestInfoStream.testTestPointsOn`.
fn test_test_points_on() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(None);
    let seen_test_point = Arc::new(AtomicBool::new(false));
    iwc.set_info_stream(Arc::new(SeenTestPoint(Arc::clone(&seen_test_point))))?;
    let iw = IndexWriter::new_with_test_points(dir.as_directory(), iwc)?;
    iw.add_document(&Document::new())?;
    iw.close()?;
    drop(iw);
    dir.close()?;
    Assert::assert_true(seen_test_point.load(Ordering::SeqCst));
    Ok(())
}

/// The anonymous `InfoStream` of `testMergeSmallIndex`.
struct ComponentLog {
    components: HashSet<&'static str>,
    info_stream: Arc<Mutex<Vec<String>>>,
}

impl InfoStream for ComponentLog {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestInfoStream$4"
    }

    fn close(&self) {}

    fn message(&self, component: &str, message: &str) {
        if self.components.contains(component) {
            self.info_stream
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(format!("[{component}] {message}\n"));
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        self.components.contains(component)
    }
}

/// Port of `TestInfoStream.testMergeSmallIndex`.
fn test_merge_small_index() -> Result<()> {
    // examine info stream output from IW, MP, MS
    let iwc = IndexWriterConfig::with_analyzer(None);
    let components: HashSet<&'static str> = ["IW", "MP", "MS", "BD", "BU"].into_iter().collect();
    let info_stream: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    iwc.set_info_stream(Arc::new(ComponentLog {
        components,
        info_stream: Arc::clone(&info_stream),
    }))?;
    {
        // try (Directory dir = newDirectory(); IndexWriter iw = new IndexWriter(dir, iwc))
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let body = (|| -> Result<()> {
            iw.add_document(&Document::new())?;
            iw.commit()?;
            iw.add_document(&Document::new())?;
            iw.add_document(&Document::new())?;
            let reader = directory_reader::open_with_writer(&*iw)?;
            let ir = Arc::clone(&reader) as Arc<dyn IndexReader>;
            let deleted = iw.try_delete_document(&ir, 2);
            let closed = reader.close();
            Assert::assert_true(deleted? > 0);
            closed?;
            iw.commit()?;
            iw.force_merge(1)?;
            Ok(())
        })();
        let mut first: Option<LuceneError> = body.err();
        if let Err(e) = iw.close() {
            first.get_or_insert(e);
        }
        drop(iw);
        if let Err(e) = dir.close() {
            first.get_or_insert(e);
        }
        if let Some(e) = first {
            return Err(e);
        }
    }
    let mut found_init = false;
    let mut flushed_count = 0;
    let mut merged_count = 0;
    for message in info_stream.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        // we don't want to be printing full diagnostics every time a segment is mentioned in the log,
        // only when it is first flushed
        if message.contains("diagnostics") {
            if message.starts_with("[IW] publishFlushedSegment") {
                flushed_count += 1;
            } else if message.starts_with("[IW] merged new segment") {
                merged_count += 1;
            } else {
                Assert::fail(&format!("message contains diagnostics: {message}"));
            }
        }
        if message.contains("init segments") {
            assert_equals(&"[IW] init segments: \n".to_string(), message);
            found_init = true;
        }
        // System.out.print(message);
    }
    Assert::assert_true_msg("init message not found", found_init);
    assert_equals(&2, &flushed_count);
    assert_equals(&1, &merged_count);
    Ok(())
}
