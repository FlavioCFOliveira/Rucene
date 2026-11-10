//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTerms.java`.

use std::cmp::Ordering;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Field, Store, TextField};
use rucene::error::Result;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::IndexReader;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::canned_binary_token_stream::{
    BinaryToken, CannedBinaryTokenStream,
};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};
use crate::support::npe;

/// `TestTerms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTerms",
        vec![
            test("testTermMinMaxBasic", test_term_min_max_basic),
            test("testTermMinMaxRandom", test_term_min_max_random),
        ],
    )
}

/// Port of `TestTerms.testTermMinMaxBasic`.
fn test_term_min_max_basic() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c cc ddd",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let terms = MultiTerms::get_terms(&r, "field")?.unwrap_or_else(|| npe("terms"));
    assert_equals(&Some(BytesRef::from_char_sequence("a")), &terms.min()?);
    assert_equals(&Some(BytesRef::from_char_sequence("ddd")), &terms.max()?);
    r.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestTerms.testTermMinMaxRandom`.
fn test_term_min_max_random() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(100);
    let mut min_term: Option<BytesRef> = None;
    let mut max_term: Option<BytesRef> = None;
    for _ in 0..num_docs {
        let mut doc = Document::new();
        // System.out.println("  doc " + i);
        let num_tokens = LuceneTestCase::at_least(10);
        let mut tokens = Vec::with_capacity(num_tokens as usize);
        for _ in 0..num_tokens {
            let mut bytes = vec![0u8; TestUtil::next_int(&*random(), 1, 20) as usize];
            random().next_bytes(&mut bytes);
            let token_bytes = BytesRef::new(bytes);
            // System.out.println("    token " + tokenBytes);
            if min_term
                .as_ref()
                .is_none_or(|m| token_bytes.cmp(m) == Ordering::Less)
            {
                // System.out.println("      ** new min");
                min_term = Some(token_bytes.clone());
            }
            if max_term
                .as_ref()
                .is_none_or(|m| token_bytes.cmp(m) == Ordering::Greater)
            {
                // System.out.println("      ** new max");
                max_term = Some(token_bytes.clone());
            }
            tokens.push(BinaryToken::new(token_bytes));
        }
        let field = Field::new_with_token_stream(
            "field",
            Arc::new(AtomicRefCell::new(CannedBinaryTokenStream::new(tokens))),
            TextField::type_not_stored().clone(),
        )?;
        doc.add(Box::new(field));
        w.add_document(&doc)?;
    }

    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let terms = MultiTerms::get_terms(&r, "field")?.unwrap_or_else(|| npe("terms"));
    assert_equals(&min_term, &terms.min()?);
    assert_equals(&max_term, &terms.max()?);

    r.close()?;
    w.close()?;
    dir.close()
}
