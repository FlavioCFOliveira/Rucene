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

//! Port of `Test02Boolean.java` (its `main`, a JUnit 3 text runner, is no
//! test method).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene_queryparser::surround::query::basic_query_factory::BasicQueryFactory;

use super::boolean_query_test_facade::BooleanQueryTestFacade;
use super::single_field_test_db::SingleFieldTestDb;
use crate::qp_support::core_support::random;
use crate::qp_support::jdk_default_hooks;
use crate::qp_support::search_support::{fixture_class, ftest, set_up};

/// `final String fieldName = "bi"`.
const FIELD_NAME: &str = "bi";
/// `boolean verbose = false`.
const VERBOSE: bool = false;
/// `int maxBasicQueries = 16`.
const MAX_BASIC_QUERIES: i32 = 16;

/// `docs1`.
const DOCS1: [&str; 5] = [
    "word1 word2 word3",
    "word4 word5",
    "ord1 ord2 ord3",
    "orda1 orda2 orda3 word2 worda3",
    "a c e a b c",
];

/// The instance of `Test02Boolean`: `SingleFieldTestDb db1`.
#[derive(Default)]
pub struct Test02Boolean {
    db1: Option<SingleFieldTestDb>,
}

/// `Test02Boolean`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(set_up(set_up_impl));
    methods.push(ftest("test02Terms01", test02_terms01));
    methods.push(ftest("test02Terms02", test02_terms02));
    methods.push(ftest("test02Terms03", test02_terms03));
    methods.push(ftest("test02Terms04", test02_terms04));
    methods.push(ftest("test02Terms05", test02_terms05));
    methods.push(ftest("test02Terms06", test02_terms06));
    methods.push(ftest("test02Terms10", test02_terms10));
    methods.push(ftest("test02Terms13", test02_terms13));
    methods.push(ftest("test02Terms14", test02_terms14));
    methods.push(ftest("test02Terms20", test02_terms20));
    methods.push(ftest("test02Terms21", test02_terms21));
    methods.push(ftest("test02Terms22", test02_terms22));
    methods.push(ftest("test02Terms23", test02_terms23));
    methods.push(ftest("test03And01", test03_and01));
    methods.push(ftest("test03And02", test03_and02));
    methods.push(ftest("test03And03", test03_and03));
    methods.push(ftest("test04Or01", test04_or01));
    methods.push(ftest("test04Or02", test04_or02));
    methods.push(ftest("test04Or03", test04_or03));
    methods.push(ftest("test05Not01", test05_not01));
    methods.push(ftest("test05Not02", test05_not02));
    methods.push(ftest("test06AndOr01", test06_and_or01));
    methods.push(ftest("test07AndOrNot02", test07_and_or_not02));
    fixture_class::<Test02Boolean>(
        "org.apache.lucene.queryparser.surround.query.Test02Boolean",
        methods,
    )
}

fn set_up_impl(t: &mut Test02Boolean) -> Result<()> {
    t.db1 = Some(SingleFieldTestDb::new(&*random(), &DOCS1, FIELD_NAME));
    Ok(())
}

/// `normalTest1(String query, int[] expdnrs)`.
fn normal_test1(t: &Test02Boolean, query: &str, expdnrs: &[i32]) -> Result<()> {
    let db1 = t.db1.as_ref().expect("setUp ran");
    let mut tbq = BooleanQueryTestFacade::new(
        query,
        expdnrs,
        db1,
        FIELD_NAME,
        BasicQueryFactory::with_max_basic_queries(MAX_BASIC_QUERIES),
    );
    tbq.set_verbose(VERBOSE);
    tbq.do_test()
}

fn test02_terms01(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "word1", &expdnrs)
}

fn test02_terms02(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 3] = [0, 1, 3];
    normal_test1(t, "word*", &expdnrs)
}

fn test02_terms03(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [2];
    normal_test1(t, "ord2", &expdnrs)
}

fn test02_terms04(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    normal_test1(t, "kxork*", &expdnrs)
}

fn test02_terms05(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 3] = [0, 1, 3];
    normal_test1(t, "wor*", &expdnrs)
}

fn test02_terms06(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    normal_test1(t, "ab", &expdnrs)
}

fn test02_terms10(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    normal_test1(t, "abc?", &expdnrs)
}

fn test02_terms13(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 3] = [0, 1, 3];
    normal_test1(t, "word?", &expdnrs)
}

fn test02_terms14(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 3] = [0, 1, 3];
    normal_test1(t, "w?rd?", &expdnrs)
}

fn test02_terms20(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 3] = [0, 1, 3];
    normal_test1(t, "w*rd?", &expdnrs)
}

fn test02_terms21(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    normal_test1(t, "w*rd??", &expdnrs)
}

fn test02_terms22(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    normal_test1(t, "w*?da?", &expdnrs)
}

fn test02_terms23(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    normal_test1(t, "w?da?", &expdnrs)
}

fn test03_and01(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "word1 AND word2", &expdnrs)
}

fn test03_and02(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    normal_test1(t, "word* and ord*", &expdnrs)
}

fn test03_and03(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "and(word1,word2)", &expdnrs)
}

fn test04_or01(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 3];
    normal_test1(t, "word1 or word2", &expdnrs)
}

fn test04_or02(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 4] = [0, 1, 2, 3];
    normal_test1(t, "word* OR ord*", &expdnrs)
}

fn test04_or03(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 3];
    normal_test1(t, "OR (word1, word2)", &expdnrs)
}

fn test05_not01(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    normal_test1(t, "word2 NOT word1", &expdnrs)
}

fn test05_not02(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "word2* not ord*", &expdnrs)
}

fn test06_and_or01(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "(word1 or ab)and or(word2,xyz, defg)", &expdnrs)
}

fn test07_and_or_not02(t: &mut Test02Boolean) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    normal_test1(t, "or( word2* not ord*, and(xyz,def))", &expdnrs)
}
