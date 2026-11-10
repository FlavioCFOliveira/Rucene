//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFlex.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, SeekStatus};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::is_instance;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};
use crate::support::npe;

/// `TestFlex`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFlex",
        vec![
            test("testNonFlex", test_non_flex),
            test("testTermOrd", test_term_ord),
        ],
    )
}

/// Test non-flex API emulated on flex index
/// Port of `TestFlex.testNonFlex`.
fn test_non_flex() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();

    const DOC_COUNT: i32 = 177;

    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_max_buffered_docs(7)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(d.as_directory(), iwc)?;

    for iter in 0..2 {
        if iter == 0 {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "field1",
                "this is field1",
                Store::NO,
            )));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "field2",
                "this is field2",
                Store::NO,
            )));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "field3",
                "aaa",
                Store::NO,
            )));
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "field4",
                "bbb",
                Store::NO,
            )));
            for _ in 0..DOC_COUNT {
                w.add_document(&doc)?;
            }
        } else {
            w.force_merge(1)?;
        }

        let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;

        let mut terms = MultiTerms::get_terms(&r, "field3")?
            .unwrap_or_else(|| npe("terms"))
            .iterator()?;
        assert_equals(
            &SeekStatus::END,
            &terms.seek_ceil(&BytesRef::from_char_sequence("abc"))?,
        );
        r.close()?;
    }

    w.close()?;
    d.close()
}

/// Port of `TestFlex.testTermOrd`.
fn test_term_ord() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_codec(TestUtil::always_postings_format(
        TestUtil::get_default_postings_format(),
    ))?;
    let w = IndexWriter::new(d.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "f",
        "a b c",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let mut terms = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?
        .terms("f")?
        .unwrap_or_else(|| npe("terms"))
        .iterator()?;
    Assert::assert_true(terms.next()?.is_some());
    match terms.ord() {
        Ok(ord) => assert_equals(&0, &ord),
        // ok -- codec is not required to support this op
        Err(e) if is_instance(&e, "java.lang.UnsupportedOperationException") => {}
        Err(e) => return Err(e),
    }
    r.close()?;
    w.close()?;
    d.close()
}
