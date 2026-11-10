//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSumDocFreq.java`.
//!
//! Tests [`Terms::sum_doc_freq`] (`Terms#getSumDocFreq()`).
//!
//! @lucene.experimental

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::field_infos::FieldInfos;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test, verbose};
use crate::support::{field_mut, npe};

/// `TestSumDocFreq`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSumDocFreq",
        vec![test("testSumDocFreq", test_sum_doc_freq)],
    )
}

/// `(char) TestUtil.nextInt(random(), 'a', 'z')`.
fn random_letter() -> char {
    char::from_u32(TestUtil::next_int(&*random(), 'a' as i32, 'z' as i32) as u32)
        .expect("ASCII letter")
}

/// Port of `TestSumDocFreq.testSumDocFreq`.
fn test_sum_doc_freq() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(500);

    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "foo",
        "",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "bar",
        "",
        Store::NO,
    )));
    for i in 0..num_docs {
        field_mut(&mut doc, "id").set_string_value(i.to_string())?;
        let ch1 = random_letter();
        let ch2 = random_letter();
        field_mut(&mut doc, "foo").set_string_value(format!("{ch1} {ch2}"))?;
        let ch1 = random_letter();
        let ch2 = random_letter();
        field_mut(&mut doc, "bar").set_string_value(format!("{ch1} {ch2}"))?;
        writer.add_document(&doc)?;
    }

    let ir: Arc<dyn IndexReader> = writer.get_reader()?;

    assert_sum_doc_freq(&ir)?;
    ir.close()?;

    let num_deletions = LuceneTestCase::at_least(20);
    for _ in 0..num_deletions {
        writer.delete_documents(Term::from_text(
            "id",
            &random().next_int_bound(num_docs).to_string(),
        ))?;
    }
    writer.force_merge(1)?;
    writer.close()?;

    let ir: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    assert_sum_doc_freq(&ir)?;
    ir.close()?;
    dir.close()
}

/// `private void assertSumDocFreq(IndexReader ir)`.
fn assert_sum_doc_freq(ir: &Arc<dyn IndexReader>) -> Result<()> {
    // compute sumDocFreq across all fields
    let fields = FieldInfos::get_indexed_fields(Arc::clone(ir))?;
    for f in fields {
        let terms = MultiTerms::get_terms(ir, &f)?.unwrap_or_else(|| npe("terms"));
        let sum_doc_freq = terms.sum_doc_freq()?;
        if sum_doc_freq == -1 {
            if verbose() {
                println!("skipping field: {f}, codec does not support sumDocFreq");
            }
            continue;
        }

        let mut computed_sum_doc_freq = 0i64;
        let mut terms_enum = terms.iterator()?;
        while terms_enum.next()?.is_some() {
            computed_sum_doc_freq += i64::from(terms_enum.doc_freq()?);
        }
        assert_equals(&computed_sum_doc_freq, &sum_doc_freq);
    }
    Ok(())
}
