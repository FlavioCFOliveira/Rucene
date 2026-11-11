//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNRTReaderCleanup.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::store::Directory;
use rucene::util::Constants;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::SuppressFileSystems;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class_annotated, random, test};

/// `TestNRTReaderCleanup` (`@SuppressFileSystems("WindowsFS")`: the bug
/// doesn't happen on windows).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestNRTReaderCleanup",
        vec![test(
            "testClosingNRTReaderDoesNotCorruptYourIndex",
            test_closing_nrt_reader_does_not_corrupt_your_index,
        )],
        |b| {
            b.annotate(SuppressFileSystems {
                value: vec!["WindowsFS".to_string()],
            })
        },
    )
}

/// Port of `TestNRTReaderCleanup.testClosingNRTReaderDoesNotCorruptYourIndex`.
fn test_closing_nrt_reader_does_not_corrupt_your_index() -> Result<()> {
    // Windows disallows deleting & overwriting files still
    // open for reading:
    LuceneTestCase::assume_false("this test can't run on Windows", Constants::windows());

    let dir = LuceneTestCase::new_mock_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    let lmp = LogMergePolicy::by_doc_count();
    lmp.set_merge_factor(2)?;
    iwc.set_merge_policy(Some(Arc::new(lmp)))?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new("a", "foo".into(), Store::NO)?));
    w.add_document(&doc)?;
    w.commit()?;
    w.add_document(&doc)?;

    // Get a new reader, but this also sets off a merge:
    let r = w.get_reader()?;
    w.close()?;
    drop(w);

    // Blow away index and make a new writer:
    let directory = dir.as_directory();
    for name in directory.list_all()? {
        directory.delete_file(&name)?;
    }

    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), Arc::clone(&directory))?;
    w.add_document(&doc)?;
    w.close()?;
    drop(w);
    r.close()?;
    drop(r);
    drop(directory);
    dir.close()
}
