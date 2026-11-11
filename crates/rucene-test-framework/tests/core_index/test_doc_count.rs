//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocCount.java`.
//!
//! Tests the Terms.docCount statistic

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::field_infos::FieldInfos;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::POSTINGS_ENUM_NONE;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::util::FixedBitSet;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};

/// `TestDocCount`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocCount",
        vec![test("testSimple", test_simple)],
    )
}

/// Port of `TestDocCount.testSimple`.
fn test_simple() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    for _ in 0..num_docs {
        iw.add_document(&doc())?;
    }
    let ir: Arc<dyn IndexReader> = iw.get_reader()?;
    verify_count(&ir)?;
    ir.close()?;
    iw.force_merge(1)?;
    let ir: Arc<dyn IndexReader> = iw.get_reader()?;
    verify_count(&ir)?;
    ir.close()?;
    iw.close()?;
    dir.close()
}

/// `private Document doc()`.
fn doc() -> Document {
    let mut doc = Document::new();
    let num_fields = TestUtil::next_int(&*random(), 1, 10);
    for _ in 0..num_fields {
        // `"" + TestUtil.nextInt(random(), 'a', 'z')`: the code, as a number.
        let name = TestUtil::next_int(&*random(), 'a' as i32, 'z' as i32).to_string();
        let value = TestUtil::next_int(&*random(), 'a' as i32, 'z' as i32).to_string();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            &name,
            &value,
            Store::NO,
        )));
    }
    doc
}

/// `private void verifyCount(IndexReader ir)`.
fn verify_count(ir: &Arc<dyn IndexReader>) -> Result<()> {
    let fields = FieldInfos::get_indexed_fields(Arc::clone(ir))?;
    for field in &fields {
        let Some(terms) = MultiTerms::get_terms(ir, field)? else {
            continue;
        };
        let doc_count = terms.doc_count()?;
        let mut visited = FixedBitSet::new(ir.max_doc() as usize);
        let mut te = terms.iterator()?;
        while te.next()?.is_some() {
            let mut de = TestUtil::docs_of(&*random(), &mut *te, None, POSTINGS_ENUM_NONE)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            loop {
                let doc = de.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                visited.set(de.doc_id() as usize);
            }
        }
        assert_equals(&(visited.cardinality() as i32), &doc_count);
    }
    Ok(())
}
