//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNewestSegment.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_null, class, random, test};

/// `TestNewestSegment`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNewestSegment",
        vec![test("testNewestSegment", test_newest_segment)],
    )
}

/// Port of `TestNewestSegment.testNewestSegment`.
fn test_newest_segment() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*random(),
        )))),
    )?;
    assert_null(&writer.newest_segment()?.map(|_| ()));
    writer.close()?;
    drop(writer);
    directory.close()
}
