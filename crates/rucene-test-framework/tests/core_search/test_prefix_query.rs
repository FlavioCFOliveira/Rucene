//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPrefixQuery.java`.
//!
//! Tests `PrefixQuery` class.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::prefix_query::PrefixQuery;
use rucene::util::string_helper::StringHelper;
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{
    assert_equals, assert_equals_msg, class, random, shuffle, test, JavaHashSet,
};

/// `TestPrefixQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPrefixQuery",
        vec![
            test("testPrefixQuery", test_prefix_query),
            test("testMatchAll", test_match_all),
            test("testRandomBinaryPrefix", test_random_binary_prefix),
        ],
    )
}

/// Port of `TestPrefixQuery.testPrefixQuery`.
fn test_prefix_query() -> Result<()> {
    let directory = LuceneTestCase::new_directory();

    let categories = ["/Computers", "/Computers/Mac", "/Computers/Windows"];
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    for category in categories {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "category",
            category,
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;

    let query = PrefixQuery::new(term("category", "/Computers"))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let hits = searcher.search(Arc::new(query), 1000)?.score_docs;
    assert_equals_msg(
        Some("All documents in /Computers category and below"),
        &3,
        &hits.len(),
    );

    let query = PrefixQuery::new(term("category", "/Computers/Mac"))?;
    let hits = searcher.search(Arc::new(query), 1000)?.score_docs;
    assert_equals_msg(Some("One in /Computers/Mac"), &1, &hits.len());

    let query = PrefixQuery::new(term("category", ""))?;
    let hits = searcher.search(Arc::new(query), 1000)?.score_docs;
    assert_equals_msg(Some("everything"), &3, &hits.len());
    writer.close()?;
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestPrefixQuery.testMatchAll`.
fn test_match_all() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "field",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;

    let query = PrefixQuery::new(term("field", ""))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    assert_equals(
        &1,
        &searcher.search(Arc::new(query), 1000)?.total_hits.value(),
    );
    writer.close()?;
    drop(searcher);
    reader.close()?;
    directory.close()?;
    Ok(())
}

/// Port of `TestPrefixQuery.testRandomBinaryPrefix`.
fn test_random_binary_prefix() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_terms = LuceneTestCase::at_least(1000);
    let mut terms: JavaHashSet<BytesRef> = JavaHashSet::new();
    while (terms.size() as i32) < num_terms {
        let mut bytes = vec![0u8; TestUtil::next_int(&*random(), 1, 10) as usize];
        random().next_bytes(&mut bytes);
        terms.add(BytesRef::new(bytes));
    }

    let mut terms_list: Vec<BytesRef> = terms.to_vec();
    shuffle(&mut terms_list, &*random());
    for t in &terms_list {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field_bytes(
            "field",
            t.clone(),
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    let r = w.get_reader()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let iters = LuceneTestCase::at_least(100);
    for _ in 0..iters {
        let mut bytes = vec![0u8; random().next_int_bound(3) as usize];
        random().next_bytes(&mut bytes);
        let prefix = BytesRef::new(bytes);
        let q = PrefixQuery::new(Term::new("field", prefix.clone()))?;
        let mut count = 0;
        for t in &terms_list {
            if StringHelper::starts_with(t, &prefix) {
                count += 1;
            }
        }
        assert_equals(&count, &s.count(Arc::new(q))?);
    }

    drop(s);
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
