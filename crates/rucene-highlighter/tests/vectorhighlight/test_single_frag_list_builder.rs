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

//! Port of `TestSingleFragListBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::single_frag_list_builder::SingleFragListBuilder;

use super::abstract_test_case::{make1dmf_index, vh_class, Fixture, F};
use crate::highlighter_support::search_support::ftest;

/// `TestSingleFragListBuilder`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestSingleFragListBuilder",
        vec![
            ftest("testNullFieldFragList", test_null_field_frag_list),
            ftest("testShortFieldFragList", test_short_field_frag_list),
            ftest("testLongFieldFragList", test_long_field_frag_list),
        ],
    )
}

fn tq_a() -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(F, "a")))
}

fn test_null_field_frag_list(f: &mut Fixture) -> Result<()> {
    let sflb = SingleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl(f, tq_a(), &["b c d"])?, 100)?;
    Assert::assert_equals(0, ffl.get_frag_infos().len());
    Ok(())
}

fn test_short_field_frag_list(f: &mut Fixture) -> Result<()> {
    let sflb = SingleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl(f, tq_a(), &["a b c d"])?, 100)?;
    Assert::assert_equals(1, ffl.get_frag_infos().len());
    Assert::assert_equals(
        "subInfos=(a((0,1)))/1.0(0,2147483647)".to_string(),
        ffl.get_frag_infos()[0].to_string(),
    );
    Ok(())
}

fn test_long_field_frag_list(f: &mut Fixture) -> Result<()> {
    let sflb = SingleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(
        &fpl(
            f,
            tq_a(),
            &[
                "a b c d",
                "a b c d e f g h i",
                "j k l m n o p q r s t u v w x y z a b c",
                "d e f g",
            ],
        )?,
        100,
    )?;
    Assert::assert_equals(1, ffl.get_frag_infos().len());
    Assert::assert_equals(
        "subInfos=(a((0,1))a((8,9))a((60,61)))/3.0(0,2147483647)".to_string(),
        ffl.get_frag_infos()[0].to_string(),
    );
    Ok(())
}

fn fpl(f: &mut Fixture, query: Arc<dyn Query>, index_values: &[&str]) -> Result<FieldPhraseList> {
    make1dmf_index(f, index_values)?;
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    Ok(FieldPhraseList::new(&mut stack, &fq))
}
