//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBlendedTermQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::query_to_string;
use rucene::search::{
    boolean_rewrite, disjunction_max_rewrite, BlendedTermQueryBuilder, DisjunctionMaxRewrite, Query,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBlendedTermQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBlendedTermQuery",
        vec![
            test("testEquals", test_equals),
            test("testToString", test_to_string),
            test("testBlendedScores", test_blended_scores),
        ],
    )
}

fn test_equals() -> Result<()> {
    let t1 = Term::from_text("foo", "bar");
    let bt1 = BlendedTermQueryBuilder::new().add_term(t1.clone())?.build();
    let bt2 = BlendedTermQueryBuilder::new().add_term(t1.clone())?.build();
    QueryUtils::check_equal(&bt1, &bt2);
    let bt1 = BlendedTermQueryBuilder::new()
        .set_rewrite_method(boolean_rewrite())
        .add_term(t1.clone())?
        .build();
    let bt2 = BlendedTermQueryBuilder::new()
        .set_rewrite_method(disjunction_max_rewrite())
        .add_term(t1.clone())?
        .build();
    QueryUtils::check_unequal(&bt1, &bt2);

    let t2 = Term::from_text("foo", "baz");
    let bt1 = BlendedTermQueryBuilder::new()
        .add_term(t1.clone())?
        .add_term(t2.clone())?
        .build();
    let bt2 = BlendedTermQueryBuilder::new()
        .add_term(t2.clone())?
        .add_term(t1.clone())?
        .build();
    QueryUtils::check_equal(&bt1, &bt2);

    let boost1 = random().next_float();
    let boost2 = random().next_float();
    let bt1 = BlendedTermQueryBuilder::new()
        .add_boosted(t1.clone(), boost1)?
        .add_boosted(t2.clone(), boost2)?
        .build();
    let bt2 = BlendedTermQueryBuilder::new()
        .add_boosted(t2, boost2)?
        .add_boosted(t1, boost1)?
        .build();
    QueryUtils::check_equal(&bt1, &bt2);
    Ok(())
}

fn test_to_string() -> Result<()> {
    assert_equals(
        "Blended()",
        query_to_string(&BlendedTermQueryBuilder::new().build()).as_str(),
    );
    let t1 = Term::from_text("foo", "bar");
    assert_equals(
        "Blended(foo:bar)",
        query_to_string(&BlendedTermQueryBuilder::new().add_term(t1.clone())?.build()).as_str(),
    );
    let t2 = Term::from_text("foo", "baz");
    assert_equals(
        "Blended(foo:bar foo:baz)",
        query_to_string(
            &BlendedTermQueryBuilder::new()
                .add_term(t1.clone())?
                .add_term(t2.clone())?
                .build(),
        )
        .as_str(),
    );
    assert_equals(
        "Blended((foo:bar)^4.0 (foo:baz)^3.0)",
        query_to_string(
            &BlendedTermQueryBuilder::new()
                .add_boosted(t1, 4.0)?
                .add_boosted(t2, 3.0)?
                .build(),
        )
        .as_str(),
    );
    Ok(())
}

fn test_blended_scores() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f", "a".into(), Store::NO)?));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new("f", "b".into(), Store::NO)?));
    for _ in 0..10 {
        w.add_document(&doc)?;
    }

    let reader = w.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query: Arc<dyn Query> = Arc::new(
        BlendedTermQueryBuilder::new()
            .set_rewrite_method(Arc::new(DisjunctionMaxRewrite::new(0.0)))
            .add_term(Term::from_text("f", "a"))?
            .add_term(Term::from_text("f", "b"))?
            .build(),
    );
    let top_docs = searcher.search(query, 20)?;
    assert_equals(&11, &top_docs.total_hits.value());
    // All docs must have the same score
    for i in 0..top_docs.score_docs.len() {
        Assert::assert_equals_float_msg(
            None,
            top_docs.score_docs[0].score,
            top_docs.score_docs[i].score,
            0.0,
        );
    }

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}
