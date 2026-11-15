/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestSpanSearchEquivalence.java`: basic equivalence tests for
//! span queries.
//!
//! TODO: we could go a little crazy for a lot of these, but these are just
//! simple minimal cases in case something goes horribly wrong. Put more
//! intense tests elsewhere.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene_queries::spans::span_containing_query::SpanContainingQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_within_query::SpanWithinQuery;
use rucene_test_framework::search::search_equivalence_test_base::{
    self, assert_same_scores, assert_same_set, assert_subset_of, random_term,
    EquivalenceTestInstance,
};

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::ftest;
use crate::queries_support::span_test_util::span_query;

type F = EquivalenceTestInstance;

/// `TestSpanSearchEquivalence extends SearchEquivalenceTestBase`.
pub fn class_def() -> Arc<Class> {
    let methods = vec![
        ftest::<F>("testSpanTermVersusTerm", test_span_term_versus_term),
        ftest::<F>("testSpanOrVersusTerm", test_span_or_versus_term),
        ftest::<F>(
            "testSpanOrDoubleVersusTerm",
            test_span_or_double_versus_term,
        ),
        ftest::<F>(
            "testSpanOrVersusBooleanTerm",
            test_span_or_versus_boolean_term,
        ),
        ftest::<F>(
            "testSpanOrVersusBooleanNear",
            test_span_or_versus_boolean_near,
        ),
        ftest::<F>("testSpanNotVersusSpanTerm", test_span_not_versus_span_term),
        ftest::<F>(
            "testSpanNotNearVersusSpanTerm",
            test_span_not_near_versus_span_term,
        ),
        ftest::<F>("testSpanNotVersusSpanNear", test_span_not_versus_span_near),
        ftest::<F>(
            "testSpanNotNearVersusSpanNear",
            test_span_not_near_versus_span_near,
        ),
        ftest::<F>(
            "testSpanFirstVersusSpanTerm",
            test_span_first_versus_span_term,
        ),
        ftest::<F>("testSpanNearVersusPhrase", test_span_near_versus_phrase),
        ftest::<F>(
            "testSpanNearVersusBooleanAnd",
            test_span_near_versus_boolean_and,
        ),
        ftest::<F>(
            "testSpanNearVersusSloppySpanNear",
            test_span_near_versus_sloppy_span_near,
        ),
        ftest::<F>(
            "testSpanNearInOrderVersusOutOfOrder",
            test_span_near_in_order_versus_out_of_order,
        ),
        ftest::<F>(
            "testSpanNearIncreasingSloppiness",
            test_span_near_increasing_sloppiness,
        ),
        ftest::<F>(
            "testSpanNearIncreasingSloppiness3",
            test_span_near_increasing_sloppiness3,
        ),
        ftest::<F>(
            "testSpanNearIncreasingOrderedSloppiness",
            test_span_near_increasing_ordered_sloppiness,
        ),
        ftest::<F>(
            "testSpanNearIncreasingOrderedSloppiness3",
            test_span_near_increasing_ordered_sloppiness3,
        ),
        ftest::<F>("testSpanRangeTerm", test_span_range_term),
        ftest::<F>(
            "testSpanRangeTermIncreasingEnd",
            test_span_range_term_increasing_end,
        ),
        ftest::<F>(
            "testSpanRangeTermEverything",
            test_span_range_term_everything,
        ),
        ftest::<F>("testSpanRangeNear", test_span_range_near),
        ftest::<F>(
            "testSpanRangeNearIncreasingEnd",
            test_span_range_near_increasing_end,
        ),
        ftest::<F>(
            "testSpanRangeNearEverything",
            test_span_range_near_everything,
        ),
        ftest::<F>("testSpanFirstTerm", test_span_first_term),
        ftest::<F>(
            "testSpanFirstTermIncreasing",
            test_span_first_term_increasing,
        ),
        ftest::<F>(
            "testSpanFirstTermEverything",
            test_span_first_term_everything,
        ),
        ftest::<F>("testSpanFirstNear", test_span_first_near),
        ftest::<F>(
            "testSpanFirstNearIncreasing",
            test_span_first_near_increasing,
        ),
        ftest::<F>(
            "testSpanFirstNearEverything",
            test_span_first_near_everything,
        ),
        ftest::<F>("testSpanWithinVsNear", test_span_within_vs_near),
        ftest::<F>("testSpanWithinVsContaining", test_span_within_vs_containing),
    ];
    let mut builder = Class::builder("org.apache.lucene.queries.spans.TestSpanSearchEquivalence")
        .extends(&search_equivalence_test_base::class())
        .constructor(Constructor::no_args(EquivalenceTestInstance::default));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `spanQuery(new SpanTermQuery(t))`.
fn sst(t: &Term) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanTermQuery::new(t.clone())))
}

fn q(s: &Arc<dyn SpanQuery>) -> Arc<dyn Query> {
    Arc::clone(s) as Arc<dyn Query>
}

/// `spanQuery(new SpanNearQuery(subquery, slop, inOrder))`.
fn snear(subquery: &[Arc<dyn SpanQuery>], slop: i32, in_order: bool) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanNearQuery::new(
        subquery.to_vec(),
        slop,
        in_order,
    )?)))
}

/// SpanTermQuery(A) = TermQuery(A)
fn test_span_term_versus_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    assert_same_scores(Arc::new(TermQuery::new(t1.clone())), q(&sst(&t1)))
}

/// SpanOrQuery(A) = SpanTermQuery(A)
fn test_span_or_versus_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let term = sst(&t1);
    assert_same_set(
        q(&span_query(Arc::new(SpanOrQuery::new(vec![Arc::clone(
            &term,
        )])?))),
        q(&term),
    )
}

/// SpanOrQuery(A, A) = SpanTermQuery(A)
fn test_span_or_double_versus_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let term = sst(&t1);
    assert_same_set(
        q(&span_query(Arc::new(SpanOrQuery::new(vec![
            Arc::clone(&term),
            Arc::clone(&term),
        ])?))),
        q(&term),
    )
}

/// SpanOrQuery(A, B) = (A B)
fn test_span_or_versus_boolean_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let mut q1 = BooleanQueryBuilder::new();
    q1.add(Arc::new(TermQuery::new(t1.clone())), Occur::SHOULD)?;
    q1.add(Arc::new(TermQuery::new(t2.clone())), Occur::SHOULD)?;
    let q2 = span_query(Arc::new(SpanOrQuery::new(vec![sst(&t1), sst(&t2)])?));
    assert_same_set(Arc::new(q1.build()), q(&q2))
}

/// SpanOrQuery(SpanNearQuery[A B], SpanNearQuery[C D]) = (SpanNearQuery[A B],
/// SpanNearQuery[C D])
fn test_span_or_versus_boolean_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let t4 = random_term();
    let near1 = snear(&[sst(&t1), sst(&t2)], 10, random().next_boolean())?;
    let near2 = snear(&[sst(&t3), sst(&t4)], 10, random().next_boolean())?;
    let mut q1 = BooleanQueryBuilder::new();
    q1.add(q(&near1), Occur::SHOULD)?;
    q1.add(q(&near2), Occur::SHOULD)?;
    let q2 = span_query(Arc::new(SpanOrQuery::new(vec![near1, near2])?));
    assert_same_set(Arc::new(q1.build()), q(&q2))
}

/// SpanNotQuery(A, B) ⊆ SpanTermQuery(A)
fn test_span_not_versus_span_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    assert_subset_of(
        q(&span_query(Arc::new(SpanNotQuery::new(
            sst(&t1),
            sst(&t2),
        )?))),
        q(&sst(&t1)),
    )
}

/// SpanNotQuery(A, [B C]) ⊆ SpanTermQuery(A)
fn test_span_not_near_versus_span_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let near = snear(&[sst(&t2), sst(&t3)], 10, random().next_boolean())?;
    assert_subset_of(
        q(&span_query(Arc::new(SpanNotQuery::new(sst(&t1), near)?))),
        q(&sst(&t1)),
    )
}

/// SpanNotQuery([A B], C) ⊆ SpanNearQuery([A B])
fn test_span_not_versus_span_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let near = snear(&[sst(&t1), sst(&t2)], 10, random().next_boolean())?;
    assert_subset_of(
        q(&span_query(Arc::new(SpanNotQuery::new(
            Arc::clone(&near),
            sst(&t3),
        )?))),
        q(&near),
    )
}

/// SpanNotQuery([A B], [C D]) ⊆ SpanNearQuery([A B])
fn test_span_not_near_versus_span_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let t4 = random_term();
    let near1 = snear(&[sst(&t1), sst(&t2)], 10, random().next_boolean())?;
    let near2 = snear(&[sst(&t3), sst(&t4)], 10, random().next_boolean())?;
    assert_subset_of(
        q(&span_query(Arc::new(SpanNotQuery::new(
            Arc::clone(&near1),
            near2,
        )?))),
        q(&near1),
    )
}

/// SpanFirstQuery(A, 10) ⊆ SpanTermQuery(A)
fn test_span_first_versus_span_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    assert_subset_of(
        q(&span_query(Arc::new(SpanFirstQuery::new(sst(&t1), 10)))),
        q(&sst(&t1)),
    )
}

/// SpanNearQuery([A, B], 0, true) = "A B"
fn test_span_near_versus_phrase(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let q1 = snear(&subquery, 0, true)?;
    let q2: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop_bytes(
        0,
        t1.field(),
        &[t1.bytes().clone(), t2.bytes().clone()],
    )?);
    if t1 == t2 {
        assert_same_set(q(&q1), q2)
    } else {
        assert_same_scores(q(&q1), q2)
    }
}

/// SpanNearQuery([A, B], ∞, false) = +A +B
fn test_span_near_versus_boolean_and(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let q1 = snear(&subquery, i32::MAX, false)?;
    let mut q2 = BooleanQueryBuilder::new();
    q2.add(Arc::new(TermQuery::new(t1)), Occur::MUST)?;
    q2.add(Arc::new(TermQuery::new(t2)), Occur::MUST)?;
    assert_same_set(q(&q1), Arc::new(q2.build()))
}

/// SpanNearQuery([A B], 0, false) ⊆ SpanNearQuery([A B], 1, false)
fn test_span_near_versus_sloppy_span_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let q1 = snear(&subquery, 0, false)?;
    let q2 = snear(&subquery, 1, false)?;
    assert_subset_of(q(&q1), q(&q2))
}

/// SpanNearQuery([A B], 3, true) ⊆ SpanNearQuery([A B], 3, false)
fn test_span_near_in_order_versus_out_of_order(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let q1 = snear(&subquery, 3, true)?;
    let q2 = snear(&subquery, 3, false)?;
    assert_subset_of(q(&q1), q(&q2))
}

/// SpanNearQuery([A B], N, false) ⊆ SpanNearQuery([A B], N+1, false)
fn test_span_near_increasing_sloppiness(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    for i in 0..10 {
        let q1 = snear(&subquery, i, false)?;
        let q2 = snear(&subquery, i + 1, false)?;
        assert_subset_of(q(&q1), q(&q2))?;
    }
    Ok(())
}

/// SpanNearQuery([A B C], N, false) ⊆ SpanNearQuery([A B C], N+1, false)
fn test_span_near_increasing_sloppiness3(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let subquery = [sst(&t1), sst(&t2), sst(&t3)];
    for i in 0..10 {
        let q1 = snear(&subquery, i, false)?;
        let q2 = snear(&subquery, i + 1, false)?;
        assert_subset_of(q(&q1), q(&q2))?;
    }
    Ok(())
}

/// SpanNearQuery([A B], N, true) ⊆ SpanNearQuery([A B], N+1, true)
fn test_span_near_increasing_ordered_sloppiness(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    for i in 0..10 {
        let q1 = snear(&subquery, i, false)?;
        let q2 = snear(&subquery, i + 1, false)?;
        assert_subset_of(q(&q1), q(&q2))?;
    }
    Ok(())
}

/// SpanNearQuery([A B C], N, true) ⊆ SpanNearQuery([A B C], N+1, true)
fn test_span_near_increasing_ordered_sloppiness3(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let t3 = random_term();
    let subquery = [sst(&t1), sst(&t2), sst(&t3)];
    for i in 0..10 {
        let q1 = snear(&subquery, i, true)?;
        let q2 = snear(&subquery, i + 1, true)?;
        assert_subset_of(q(&q1), q(&q2))?;
    }
    Ok(())
}

/// `spanQuery(new SpanPositionRangeQuery(match, start, end))`.
fn srange(m: Arc<dyn SpanQuery>, start: i32, end: i32) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanPositionRangeQuery::new(m, start, end)))
}

/// SpanPositionRangeQuery(A, M, N) ⊆ TermQuery(A)
fn test_span_range_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    for i in 0..5 {
        for j in 0..5 {
            let q1 = q(&srange(sst(&t1), i, i + j));
            let q2: Arc<dyn Query> = Arc::new(TermQuery::new(t1.clone()));
            assert_subset_of(q1, q2)?;
        }
    }
    Ok(())
}

/// SpanPositionRangeQuery(A, M, N) ⊆ SpanFirstQuery(A, M, N+1)
fn test_span_range_term_increasing_end(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    for i in 0..5 {
        for j in 0..5 {
            let q1 = q(&srange(sst(&t1), i, i + j));
            let q2 = q(&srange(sst(&t1), i, i + j + 1));
            assert_subset_of(q1, q2)?;
        }
    }
    Ok(())
}

/// SpanPositionRangeQuery(A, 0, ∞) = TermQuery(A)
fn test_span_range_term_everything(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let q1 = q(&srange(sst(&t1), 0, i32::MAX));
    let q2: Arc<dyn Query> = Arc::new(TermQuery::new(t1));
    assert_same_set(q1, q2)
}

/// SpanPositionRangeQuery([A B], M, N) ⊆ SpanNearQuery([A B])
fn test_span_range_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    for i in 0..5 {
        for j in 0..5 {
            let q1 = q(&srange(Arc::clone(&near_query), i, i + j));
            let q2 = q(&near_query);
            assert_subset_of(q1, q2)?;
        }
    }
    Ok(())
}

/// SpanPositionRangeQuery([A B], M, N) ⊆ SpanFirstQuery([A B], M, N+1)
fn test_span_range_near_increasing_end(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    for i in 0..5 {
        for j in 0..5 {
            let q1 = q(&srange(Arc::clone(&near_query), i, i + j));
            let q2 = q(&srange(Arc::clone(&near_query), i, i + j + 1));
            assert_subset_of(q1, q2)?;
        }
    }
    Ok(())
}

/// SpanPositionRangeQuery([A B], ∞) = SpanNearQuery([A B])
fn test_span_range_near_everything(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    let q1 = q(&srange(Arc::clone(&near_query), 0, i32::MAX));
    let q2 = q(&near_query);
    assert_same_set(q1, q2)
}

/// `spanQuery(new SpanFirstQuery(match, end))`.
fn sfirst(m: Arc<dyn SpanQuery>, end: i32) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanFirstQuery::new(m, end)))
}

/// SpanFirstQuery(A, N) ⊆ TermQuery(A)
fn test_span_first_term(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    for i in 0..10 {
        let q1 = q(&sfirst(sst(&t1), i));
        let q2: Arc<dyn Query> = Arc::new(TermQuery::new(t1.clone()));
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// SpanFirstQuery(A, N) ⊆ SpanFirstQuery(A, N+1)
fn test_span_first_term_increasing(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    for i in 0..10 {
        let q1 = q(&sfirst(sst(&t1), i));
        let q2 = q(&sfirst(sst(&t1), i + 1));
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// SpanFirstQuery(A, ∞) = TermQuery(A)
fn test_span_first_term_everything(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let q1 = q(&sfirst(sst(&t1), i32::MAX));
    let q2: Arc<dyn Query> = Arc::new(TermQuery::new(t1));
    assert_same_set(q1, q2)
}

/// SpanFirstQuery([A B], N) ⊆ SpanNearQuery([A B])
fn test_span_first_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    for i in 0..10 {
        let q1 = q(&sfirst(Arc::clone(&near_query), i));
        let q2 = q(&near_query);
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// SpanFirstQuery([A B], N) ⊆ SpanFirstQuery([A B], N+1)
fn test_span_first_near_increasing(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    for i in 0..10 {
        let q1 = q(&sfirst(Arc::clone(&near_query), i));
        let q2 = q(&sfirst(Arc::clone(&near_query), i + 1));
        assert_subset_of(q1, q2)?;
    }
    Ok(())
}

/// SpanFirstQuery([A B], ∞) = SpanNearQuery([A B])
fn test_span_first_near_everything(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;
    let q1 = q(&sfirst(Arc::clone(&near_query), i32::MAX));
    let q2 = q(&near_query);
    assert_same_set(q1, q2)
}

/// SpanWithinQuery(A, B) ⊆ SpanNearQuery(A)
fn test_span_within_vs_near(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;

    let t3 = random_term();
    let term_query = sst(&t3);
    let q1 = q(&span_query(Arc::new(SpanWithinQuery::new(
        near_query,
        Arc::clone(&term_query),
    )?)));
    assert_subset_of(q1, q(&term_query))
}

/// SpanWithinQuery(A, B) = SpanContainingQuery(A, B)
fn test_span_within_vs_containing(_f: &mut F) -> Result<()> {
    let t1 = random_term();
    let t2 = random_term();
    let subquery = [sst(&t1), sst(&t2)];
    let near_query = snear(&subquery, 10, true)?;

    let t3 = random_term();
    let term_query = sst(&t3);
    let q1 = q(&span_query(Arc::new(SpanWithinQuery::new(
        Arc::clone(&near_query),
        Arc::clone(&term_query),
    )?)));
    let q2 = q(&span_query(Arc::new(SpanContainingQuery::new(
        near_query, term_query,
    )?)));
    assert_same_set(q1, q2)
}
