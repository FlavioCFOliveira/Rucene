//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermStates.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::terms::{Term, TermStates};
use rucene::index::IndexReader;
use rucene::search::IndexSearcher;
use rucene::util::IOUtils;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestTermStates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermStates",
        vec![test(
            "testToStringOnNullTermState",
            test_to_string_on_null_term_state,
        )],
    )
}

/// Port of `TestTermStates.testToStringOnNullTermState`.
fn test_to_string_on_null_term_state() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let s = IndexSearcher::new(Arc::clone(&r))?;
    let states = TermStates::build(&s, &Term::from_text("foo", "bar"), random().next_boolean())?;
    assert_equals("TermStates\n  state=null\n", states.to_string().as_str());
    IOUtils::close([
        Some(Box::new(|| r.close()) as Box<dyn FnOnce() -> Result<()>>),
        Some(Box::new(|| w.close())),
        Some(Box::new(|| dir.close())),
    ])
}
