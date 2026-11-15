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

//! Port of `TestFunctionScoreExplanations.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::double_values_source;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity};
use rucene::search::term_query::TermQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_test_framework::search::base_explanation_test_case::{
    self, qtest, state, ExplanationTestInstance, FIELD,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::test;

/// `TestFunctionScoreExplanations`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.function.TestFunctionScoreExplanations")
        .extends(&base_explanation_test_case::class())
        .constructor(Constructor::no_args(ExplanationTestInstance::default))
        .method(test("testOneTerm", test_one_term))
        .method(test("testBoost", test_boost))
        .method(test("testTopLevelBoost", test_top_level_boost))
        .method(test(
            "testExplanationsIncludingScore",
            test_explanations_including_score,
        ))
        .method(test("testSubExplanations", test_sub_explanations))
        .build()
}

fn w1() -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(FIELD, "w1")))
}

fn test_one_term() -> Result<()> {
    let q = w1();
    let fsq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        q,
        double_values_source::constant(5.0),
    ));
    qtest(fsq, &[0, 1, 2, 3])
}

fn test_boost() -> Result<()> {
    let q = w1();
    let csq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        q,
        double_values_source::constant(5.0),
    ));
    qtest(Arc::new(BoostQuery::new(csq, 4.0)?), &[0, 1, 2, 3])
}

fn test_top_level_boost() -> Result<()> {
    let q = w1();
    let csq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        q,
        double_values_source::constant(5.0),
    ));
    let mut bq_b = BooleanQueryBuilder::new();
    bq_b.add(Arc::new(MatchAllDocsQuery::instance()), Occur::MUST)?;
    bq_b.add(csq, Occur::MUST)?;
    let bq: Arc<dyn Query> = Arc::new(bq_b.build());
    qtest(Arc::new(BoostQuery::new(bq, 6.0)?), &[0, 1, 2, 3])
}

fn test_explanations_including_score() -> Result<()> {
    let q = w1();
    let fsq: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        Arc::clone(&q),
        double_values_source::scores(),
    ));
    qtest(Arc::clone(&fsq), &[0, 1, 2, 3])?;

    let (e1, e) = {
        let guard = state();
        let searcher = &guard
            .as_ref()
            .expect("beforeClassTestExplanations ran")
            .searcher;
        (searcher.explain(q, 0)?, searcher.explain(fsq, 0)?)
    };
    Assert::assert_equals(e.value(), e1.value());
    Assert::assert_equals(&e.details()[0], &e1);
    Ok(())
}

fn test_sub_explanations() -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        Arc::new(MatchAllDocsQuery::instance()),
        double_values_source::constant(5.0),
    ));
    let reader = {
        let guard = state();
        Arc::clone(
            &guard
                .as_ref()
                .expect("beforeClassTestExplanations ran")
                .reader,
        )
    };
    let mut searcher = LuceneTestCase::new_searcher(reader);
    searcher.set_similarity(Arc::new(BM25Similarity::new()));

    let expl = searcher.explain(Arc::clone(&query), 0)?;
    let sub_expl = &expl.details()[0];
    Assert::assert_equals_str(None, Some("constant(5.0)"), Some(sub_expl.description()));
    Assert::assert_equals(0usize, sub_expl.details().len());

    let query: Arc<dyn Query> = Arc::new(BoostQuery::new(query, 2.0)?);
    let expl = searcher.explain(Arc::clone(&query), 0)?;
    Assert::assert_equals(2usize, expl.details().len());
    // function
    Assert::assert_equals_float_msg(
        None,
        5.0,
        expl.details()[1].value().double_value() as f32,
        0.0,
    );
    // boost
    Assert::assert_equals_str(None, Some("boost"), Some(expl.details()[0].description()));
    Assert::assert_equals_float_msg(
        None,
        2.0,
        expl.details()[0].value().double_value() as f32,
        0.0,
    );

    searcher.set_similarity(Arc::new(ClassicSimilarity::new())); // in order to have a queryNorm != 1
    let expl = searcher.explain(query, 0)?;
    Assert::assert_equals(2usize, expl.details().len());
    // function
    Assert::assert_equals_float_msg(
        None,
        5.0,
        expl.details()[1].value().double_value() as f32,
        0.0,
    );
    // boost
    Assert::assert_equals_str(None, Some("boost"), Some(expl.details()[0].description()));
    Assert::assert_equals_float_msg(
        None,
        2.0,
        expl.details()[0].value().double_value() as f32,
        0.0,
    );
    Ok(())
}
