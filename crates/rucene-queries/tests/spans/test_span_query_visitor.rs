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

//! Port of `TestSpanQueryVisitor.java`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::phrase_query::Builder as PhraseQueryBuilder;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::term_collector;
use rucene::search::term_query::TermQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;

use crate::queries_support::core_support::{assert_equals, class, test};

fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// The static `query` of the class, built at class initialization.
fn build_query() -> Result<Arc<dyn Query>> {
    let mut inner = BooleanQueryBuilder::new();
    inner.add(tq("field1", "tm2"), Occur::SHOULD)?;
    inner.add(
        Arc::new(BoostQuery::new(tq("field1", "tm3"), 2.0)?),
        Occur::SHOULD,
    )?;

    let mut phrase = PhraseQueryBuilder::new();
    phrase.add(term("field1", "term4"))?;
    phrase.add(term("field1", "term5"))?;

    let mut boosted = BooleanQueryBuilder::new();
    boosted.add(
        Arc::new(BoostQuery::new(tq("field2", "term10"), 3.0)?),
        Occur::MUST,
    )?;

    let mut b = BooleanQueryBuilder::new();
    b.add(tq("field1", "t1"), Occur::MUST)?;
    b.add(Arc::new(inner.build()), Occur::MUST)?;
    b.add(
        Arc::new(BoostQuery::new(Arc::new(phrase.build()?), 3.0)?),
        Occur::MUST,
    )?;
    b.add(
        Arc::new(SpanNearQuery::new(
            vec![
                Arc::new(SpanTermQuery::new(term("field1", "term6"))),
                Arc::new(SpanTermQuery::new(term("field1", "term7"))),
            ],
            2,
            true,
        )?),
        Occur::MUST,
    )?;
    b.add(tq("field1", "term8"), Occur::MUST_NOT)?;
    b.add(
        Arc::new(PrefixQuery::new(term("field1", "term9"))?),
        Occur::SHOULD,
    )?;
    b.add(
        Arc::new(BoostQuery::new(Arc::new(boosted.build()), 2.0)?),
        Occur::SHOULD,
    )?;
    Ok(Arc::new(b.build()))
}

static QUERY: LazyLock<Arc<dyn Query>> =
    LazyLock::new(|| build_query().expect("INVARIANT: the static query builds"));

/// `TestSpanQueryVisitor`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanQueryVisitor",
        vec![test(
            "testExtractTermsEquivalent",
            test_extract_terms_equivalent,
        )],
    )
}

fn test_extract_terms_equivalent() -> Result<()> {
    let mut terms: HashSet<Term> = HashSet::new();
    let expected: HashSet<Term> = [
        term("field1", "t1"),
        term("field1", "tm2"),
        term("field1", "tm3"),
        term("field1", "term4"),
        term("field1", "term5"),
        term("field1", "term6"),
        term("field1", "term7"),
        term("field2", "term10"),
    ]
    .into_iter()
    .collect();
    QUERY.visit(&mut term_collector(&mut terms));
    assert_equals(&expected, &terms);
    Ok(())
}
