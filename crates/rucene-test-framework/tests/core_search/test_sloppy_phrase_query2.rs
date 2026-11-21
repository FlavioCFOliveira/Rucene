//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSloppyPhraseQuery2.java`.
//!
//! random sloppy phrase query tests

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::multi_phrase_query::{Builder as MultiPhraseQueryBuilder, MultiPhraseQuery};
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::Query;
use rucene::util::BytesRef;
use rucene_test_framework::search::search_equivalence_test_base::{assert_subset_of, random_term};
use rucene_test_framework::util::TestUtil;

use crate::core_search::test_approximation_search_equivalence::{eq_class, eq_test};
use crate::core_support::random;

/// `TestSloppyPhraseQuery2`.
pub fn class_def() -> Arc<Class> {
    eq_class(
        "org.apache.lucene.search.TestSloppyPhraseQuery2",
        vec![
            eq_test("testIncreasingSloppiness", test_increasing_sloppiness),
            eq_test(
                "testIncreasingSloppinessWithHoles",
                test_increasing_sloppiness_with_holes,
            ),
            eq_test("testIncreasingSloppiness3", test_increasing_sloppiness3),
            eq_test(
                "testIncreasingSloppiness3WithHoles",
                test_increasing_sloppiness3_with_holes,
            ),
            eq_test(
                "testRepetitiveIncreasingSloppiness",
                test_repetitive_increasing_sloppiness,
            ),
            eq_test(
                "testRepetitiveIncreasingSloppinessWithHoles",
                test_repetitive_increasing_sloppiness_with_holes,
            ),
            eq_test(
                "testRepetitiveIncreasingSloppiness3",
                test_repetitive_increasing_sloppiness3,
            ),
            eq_test(
                "testRepetitiveIncreasingSloppiness3WithHoles",
                test_repetitive_increasing_sloppiness3_with_holes,
            ),
            eq_test(
                "testRandomIncreasingSloppiness",
                test_random_increasing_sloppiness,
            ),
        ],
    )
}

/// `new PhraseQuery(slop, field, bytes...)`.
fn phrase(slop: i32, field: &str, terms: &[&BytesRef]) -> Result<Arc<dyn Query>> {
    let bytes: Vec<BytesRef> = terms.iter().map(|b| (*b).clone()).collect();
    Ok(Arc::new(PhraseQuery::with_slop_bytes(slop, field, &bytes)?))
}

/// `builder.build()` as a query.
fn built(builder: &PhraseQueryBuilder) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(builder.build()?))
}

/// "A B"~N ⊆ "A B"~N+1
///
/// Port of `TestSloppyPhraseQuery2.testIncreasingSloppiness`.
fn test_increasing_sloppiness() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    for i in 0..10 {
        let q1 = phrase(i, t1.field(), &[t1.bytes(), t2.bytes()])?;
        let q2 = phrase(i + 1, t1.field(), &[t1.bytes(), t2.bytes()])?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// same as the above with posincr
///
/// Port of `TestSloppyPhraseQuery2.testIncreasingSloppinessWithHoles`.
fn test_increasing_sloppiness_with_holes() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    for i in 0..10 {
        let mut builder = PhraseQueryBuilder::new();
        builder.add_at(t1.clone(), 0)?;
        builder.add_at(t2.clone(), 2)?;
        builder.set_slop(i);
        let q1 = built(&builder)?;
        builder.set_slop(i + 1);
        let q2 = built(&builder)?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// "A B C"~N ⊆ "A B C"~N+1
///
/// Port of `TestSloppyPhraseQuery2.testIncreasingSloppiness3`.
fn test_increasing_sloppiness3() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    for i in 0..10 {
        let q1 = phrase(i, t1.field(), &[t1.bytes(), t2.bytes(), t3.bytes()])?;
        let q2 = phrase(i + 1, t1.field(), &[t1.bytes(), t2.bytes(), t3.bytes()])?;
        assert_subset_of(Arc::clone(&q1), Arc::clone(&q2))?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// same as the above with posincr
///
/// Port of `TestSloppyPhraseQuery2.testIncreasingSloppiness3WithHoles`.
fn test_increasing_sloppiness3_with_holes() -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let pos1 = 1 + random().next_int_bound(3);
    let pos2 = pos1 + 1 + random().next_int_bound(3);
    for i in 0..10 {
        let mut builder = PhraseQueryBuilder::new();
        builder.add_at(t1.clone(), 0)?;
        builder.add_at(t2.clone(), pos1)?;
        builder.add_at(t3.clone(), pos2)?;
        builder.set_slop(i);
        let q1 = built(&builder)?;
        builder.set_slop(i + 1);
        let q2 = built(&builder)?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// "A A"~N ⊆ "A A"~N+1
///
/// Port of `TestSloppyPhraseQuery2.testRepetitiveIncreasingSloppiness`.
fn test_repetitive_increasing_sloppiness() -> Result<()> {
    let t = random_term();
    for i in 0..10 {
        let q1 = phrase(i, t.field(), &[t.bytes(), t.bytes()])?;
        let q2 = phrase(i + 1, t.field(), &[t.bytes(), t.bytes()])?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// same as the above with posincr
///
/// Port of `TestSloppyPhraseQuery2.testRepetitiveIncreasingSloppinessWithHoles`.
fn test_repetitive_increasing_sloppiness_with_holes() -> Result<()> {
    let t = random_term();
    for i in 0..10 {
        let mut builder = PhraseQueryBuilder::new();
        builder.add_at(t.clone(), 0)?;
        builder.add_at(t.clone(), 2)?;
        builder.set_slop(i);
        let q1 = built(&builder)?;
        builder.set_slop(i + 1);
        let q2 = built(&builder)?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// "A A A"~N ⊆ "A A A"~N+1
///
/// Port of `TestSloppyPhraseQuery2.testRepetitiveIncreasingSloppiness3`.
fn test_repetitive_increasing_sloppiness3() -> Result<()> {
    let t = random_term();
    for i in 0..10 {
        let q1 = phrase(i, t.field(), &[t.bytes(), t.bytes(), t.bytes()])?;
        let q2 = phrase(i + 1, t.field(), &[t.bytes(), t.bytes(), t.bytes()])?;
        assert_subset_of(Arc::clone(&q1), Arc::clone(&q2))?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// same as the above with posincr
///
/// Port of `TestSloppyPhraseQuery2.testRepetitiveIncreasingSloppiness3WithHoles`.
fn test_repetitive_increasing_sloppiness3_with_holes() -> Result<()> {
    let t = random_term();
    let pos1 = 1 + random().next_int_bound(3);
    let pos2 = pos1 + 1 + random().next_int_bound(3);
    for i in 0..10 {
        let mut builder = PhraseQueryBuilder::new();
        builder.add_at(t.clone(), 0)?;
        builder.add_at(t.clone(), pos1)?;
        builder.add_at(t.clone(), pos2)?;
        builder.set_slop(i);
        let q1 = built(&builder)?;
        builder.set_slop(i + 1);
        let q2 = built(&builder)?;
        assert_subset_of(Arc::clone(&q1), Arc::clone(&q2))?;
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// MultiPhraseQuery~N ⊆ MultiPhraseQuery~N+1
///
/// Port of `TestSloppyPhraseQuery2.testRandomIncreasingSloppiness`.
fn test_random_increasing_sloppiness() -> Result<()> {
    let seed = random().next_long();
    for i in 0..10 {
        let q1 = random_phrase_query(seed)?;
        let q2 = random_phrase_query(seed)?;
        let q1 = MultiPhraseQueryBuilder::from_query(&q1)
            .set_slop(i)?
            .build();
        let q2 = MultiPhraseQueryBuilder::from_query(&q2)
            .set_slop(i + 1)?
            .build();
        assert_subset_of(Arc::new(q1), Arc::new(q2))?;
    }
    Ok(())
}

/// `private MultiPhraseQuery randomPhraseQuery(long seed)`.
fn random_phrase_query(seed: i64) -> Result<MultiPhraseQuery> {
    let random = JdkRandom::new(seed);
    let length = TestUtil::next_int(&random, 2, 5);
    let mut pqb = MultiPhraseQueryBuilder::new();
    let mut position = 0;
    for _i in 0..length {
        let depth = TestUtil::next_int(&random, 1, 3);
        let mut terms = Vec::with_capacity(depth as usize);
        for _j in 0..depth {
            let c = char::from_u32(TestUtil::next_int(&random, 'a' as i32, 'z' as i32) as u32)
                .expect("a to z");
            terms.push(Term::from_text("field", &c.to_string()));
        }
        pqb.add_at(terms, position)?;
        position += TestUtil::next_int(&random, 1, 3);
    }
    Ok(pqb.build())
}
