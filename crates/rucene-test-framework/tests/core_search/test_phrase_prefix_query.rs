//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPhrasePrefixQuery.java`.
//!
//! This class tests PhrasePrefixQuery class.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, Term};
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::term;
use crate::core_support::{assert_equals, class, random, test};

/// `TestPhrasePrefixQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPhrasePrefixQuery",
        vec![test("testPhrasePrefix", test_phrase_prefix)],
    )
}

fn body_doc(text: &str) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        text,
        Store::YES,
    )));
    doc
}

/// Port of `TestPhrasePrefixQuery.testPhrasePrefix`.
fn test_phrase_prefix() -> Result<()> {
    let mut index_store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    let doc1 = body_doc("blueberry pie");
    let doc2 = body_doc("blueberry strudel");
    let doc3 = body_doc("blueberry pizza");
    let doc4 = body_doc("blueberry chewing gum");
    let doc5 = body_doc("piccadilly circus");
    writer.add_document(&doc1)?;
    writer.add_document(&doc2)?;
    writer.add_document(&doc3)?;
    writer.add_document(&doc4)?;
    writer.add_document(&doc5)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // PhrasePrefixQuery query1 = new PhrasePrefixQuery();
    let mut query1builder = MultiPhraseQueryBuilder::new();
    // PhrasePrefixQuery query2 = new PhrasePrefixQuery();
    let mut query2builder = MultiPhraseQueryBuilder::new();
    query1builder.add(term("body", "blueberry"))?;
    query2builder.add(term("body", "strawberry"))?;

    let mut terms_with_prefix: Vec<Term> = Vec::new();

    // this TermEnum gives "piccadilly", "pie" and "pizza".
    let prefix = "pi";
    let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut te = MultiTerms::get_terms(&ir, "body")?
        .expect("body is indexed")
        .iterator()?;
    te.seek_ceil(&BytesRef::from_char_sequence(prefix))?;
    loop {
        let s = te
            .term()?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?
            .utf8_to_string()?;
        if s.starts_with(prefix) {
            terms_with_prefix.push(term("body", &s));
        } else {
            break;
        }
        if te.next()?.is_none() {
            break;
        }
    }

    query1builder.add_terms(terms_with_prefix.clone())?;
    query2builder.add_terms(terms_with_prefix)?;

    let result = searcher
        .search(Arc::new(query1builder.build()), 1000)?
        .score_docs;
    assert_equals(&2, &result.len());

    let result = searcher
        .search(Arc::new(query2builder.build()), 1000)?
        .score_docs;
    assert_equals(&0, &result.len());
    drop(searcher);
    reader.close()?;
    index_store.close()?;
    Ok(())
}
