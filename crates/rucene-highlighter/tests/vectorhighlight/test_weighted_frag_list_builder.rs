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

//! Port of `TestWeightedFragListBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::weighted_frag_list_builder::WeightedFragListBuilder;

use super::abstract_test_case::{make_index_long_mv, pq_f, tq, vh_class, Fixture, F};
use crate::highlighter_support::search_support::ftest;

/// `TestWeightedFragListBuilder`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestWeightedFragListBuilder",
        vec![
            ftest("test2WeightedFragList", test_2_weighted_frag_list),
            ftest("test2SubInfos", test_2_sub_infos),
        ],
    )
}

fn test_2_weighted_frag_list(f: &mut Fixture) -> Result<()> {
    test_case(
        f,
        pq_f(&["the", "both"]),
        100,
        "subInfos=(theboth((195,203)))/0.8679108(149,249)",
        0.8679108,
    )
}

fn test_2_sub_infos(f: &mut Fixture) -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["the", "both"]), Occur::MUST)?;
    query.add(tq("examples"), Occur::MUST)?;

    test_case(
        f,
        Arc::new(query.build()),
        1000,
        "subInfos=(examples((19,27))examples((66,74))theboth((195,203)))/1.8411169(0,1000)",
        1.8411169,
    )
}

fn test_case(
    f: &mut Fixture,
    query: Arc<dyn Query>,
    frag_char_size: i32,
    expected_frag_info: &str,
    expected_total_sub_info_boost: f64,
) -> Result<()> {
    make_index_long_mv(f)?;

    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    let fpl = FieldPhraseList::new(&mut stack, &fq);
    let wflb = WeightedFragListBuilder::new();
    let ffl = wflb.create_field_frag_list(&fpl, frag_char_size)?;
    Assert::assert_equals(1, ffl.get_frag_infos().len());
    Assert::assert_equals(
        expected_frag_info.to_string(),
        ffl.get_frag_infos()[0].to_string(),
    );

    let mut total_sub_info_boost = 0f32;
    for info in ffl.get_frag_infos() {
        for sub_info in info.get_sub_infos() {
            total_sub_info_boost += sub_info.boost();
        }
    }
    Assert::assert_equals_double_msg(
        None,
        expected_total_sub_info_boost,
        f64::from(total_sub_info_boost),
        0.0000001,
    );
    Ok(())
}
