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

//! Port of `TestScoreOrderFragmentsBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene_highlighter::vectorhighlight::field_frag_list::FieldFragList;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::fragments_builder::FragmentsBuilder;
use rucene_highlighter::vectorhighlight::score_order_fragments_builder::ScoreOrderFragmentsBuilder;
use rucene_highlighter::vectorhighlight::simple_frag_list_builder::SimpleFragListBuilder;

use super::abstract_test_case::{make1d1f_index, vh_class, Fixture, F};
use crate::highlighter_support::search_support::ftest;

/// `TestScoreOrderFragmentsBuilder`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestScoreOrderFragmentsBuilder",
        vec![ftest("test3Frags", test_3_frags)],
    )
}

fn test_3_frags(f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(Arc::new(TermQuery::new(Term::from_text(F, "a"))), Occur::SHOULD)?;
    query.add(Arc::new(TermQuery::new(Term::from_text(F, "c"))), Occur::SHOULD)?;

    let mut ffl = ffl(
        f,
        Arc::new(query.build()),
        "a b b b b b b b b b b b a b a b b b b b c a a b b",
    )?;
    let sofb = ScoreOrderFragmentsBuilder::new();
    let frags = sofb
        .create_fragments(f.reader(), 0, F, &mut *ffl, 3)?
        .expect("fragments");
    Assert::assert_equals(3, frags.len());
    // check score order
    Assert::assert_equals("<b>c</b> <b>a</b> <b>a</b> b b".to_string(), frags[0].clone());
    Assert::assert_equals("b b <b>a</b> b <b>a</b> b b b b b c".to_string(), frags[1].clone());
    Assert::assert_equals("<b>a</b> b b b b b b b b b b".to_string(), frags[2].clone());
    Ok(())
}

fn ffl(f: &mut Fixture, query: Arc<dyn Query>, index_value: &str) -> Result<Box<dyn FieldFragList>> {
    make1d1f_index(f, index_value)?;
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    let fpl = FieldPhraseList::new(&mut stack, &fq);
    SimpleFragListBuilder::new().create_field_frag_list(&fpl, 20)
}
