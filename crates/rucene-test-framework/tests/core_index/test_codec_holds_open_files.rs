//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCodecHoldsOpenFiles.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, IntPoint, NumericDocValuesField, TextField};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};
use crate::support::set_check_index_on_close;

/// `TestCodecHoldsOpenFiles`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCodecHoldsOpenFiles",
        vec![test("test", test_)],
    )
}

/// Port of `TestCodecHoldsOpenFiles.test`.
fn test_() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    set_check_index_on_close(&d, false);
    // we nuke files, but verify the reader still works
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_field(
            "foo",
            "bar",
            TextField::type_not_stored(),
        )));
        doc.add(Box::new(IntPoint::new("doc", &[i])?));
        doc.add(Box::new(IntPoint::new("doc2d", &[i, i])?));
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        w.add_document(&doc)?;
    }

    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.commit()?;
    w.close()?;

    let dir: Arc<dyn Directory> = d.as_directory();
    for name in dir.list_all()? {
        dir.delete_file(&name)?;
    }

    for cxt in Arc::clone(&r).leaves()? {
        TestUtil::check_reader(&(cxt.leaf_reader().as_index_reader()))?;
    }

    r.close()?;
    d.close()
}
