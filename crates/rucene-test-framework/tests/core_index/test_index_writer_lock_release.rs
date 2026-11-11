//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexWriterLockRelease.java`.
//!
//! This tests the patch for issue #1253 (LUCENE-311).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::is_instance;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestIndexWriterLockRelease`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexWriterLockRelease",
        vec![test(
            "testIndexWriterLockRelease",
            test_index_writer_lock_release,
        )],
    )
}

/// `catch (FileNotFoundException | NoSuchFileException e)`.
fn is_file_not_found(e: &LuceneError) -> bool {
    is_instance(e, "java.io.FileNotFoundException")
        || is_instance(e, "java.nio.file.NoSuchFileException")
}

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random()))
/// .setOpenMode(OpenMode.APPEND))`.
fn open_append(dir: &Arc<dyn rucene::store::Directory>) -> Result<Arc<IndexWriter>> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_open_mode(OpenMode::APPEND)?;
    IndexWriter::new(Arc::clone(dir), iwc)
}

/// Port of `TestIndexWriterLockRelease.testIndexWriterLockRelease`.
fn test_index_writer_lock_release() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "testLockRelease",
    ));
    let d = dir.as_directory();
    let result = match open_append(&d) {
        Ok(_) => Ok(()),
        Err(e) if is_file_not_found(&e) => match open_append(&d) {
            Ok(_) => Ok(()),
            Err(e1) if is_file_not_found(&e1) => Ok(()),
            Err(e1) => Err(e1),
        },
        Err(e) => Err(e),
    };
    drop(d);
    let closed = dir.close();
    result.and(closed)
}
