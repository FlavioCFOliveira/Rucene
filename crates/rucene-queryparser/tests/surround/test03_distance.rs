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

//! Port of `Test03Distance.java` (its `main`, a JUnit 3 text runner, is no
//! test method).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_queryparser::surround::query::basic_query_factory::BasicQueryFactory;

use super::boolean_query_test_facade::BooleanQueryTestFacade;
use super::exception_query_test_facade::ExceptionQueryTestFacade;
use super::single_field_test_db::SingleFieldTestDb;
use crate::qp_support::core_support::random;
use crate::qp_support::jdk_default_hooks;
use crate::qp_support::search_support::{fixture_class, ftest, set_up};

/// `boolean verbose = false`.
const VERBOSE: bool = false;
/// `int maxBasicQueries = 16`.
const MAX_BASIC_QUERIES: i32 = 16;
/// `final String fieldName = "bi"`.
const FIELD_NAME: &str = "bi";

/// `exceptionQueries`.
const EXCEPTION_QUERIES: [&str; 9] = [
    "(aa and bb) w cc",
    "(aa or bb) w (cc and dd)",
    "(aa opt bb) w cc",
    "(aa not bb) w cc",
    "(aa or bb) w (bi:cc)",
    "(aa or bb) w bi:cc",
    "(aa or bi:bb) w cc",
    "(aa or (bi:bb)) w cc",
    "(aa or (bb and dd)) w cc",
];

/// `docs1`.
const DOCS1: [&str; 5] = [
    "word1 word2 word3",
    "word4 word5",
    "ord1 ord2 ord3",
    "orda1 orda2 orda3 word2 worda3",
    "a c e a b c",
];

/// `docs2`.
const DOCS2: [&str; 3] = ["w1 w2 w3 w4 w5", "w1 w3 w2 w3", ""];

/// `docs3`.
const DOCS3: [&str; 4] = [
    "low pressure temperature inversion and rain",
    "when the temperature has a negative height above a depression no precipitation gradient is expected",
    "when the temperature has a negative height gradient above a depression no precipitation is expected",
    "",
];

/// The instance of `Test03Distance`: `db1`, `db2`, `db3`.
#[derive(Default)]
pub struct Test03Distance {
    db1: Option<SingleFieldTestDb>,
    db2: Option<SingleFieldTestDb>,
    db3: Option<SingleFieldTestDb>,
}

/// `Test03Distance`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(set_up(set_up_impl));
    methods.push(ftest("test00Exceptions", test00_exceptions));
    methods.push(ftest("test0W01", test0_w01));
    methods.push(ftest("test0N01", test0_n01));
    methods.push(ftest("test0N01r", test0_n01r));
    methods.push(ftest("test0W02", test0_w02));
    methods.push(ftest("test0W03", test0_w03));
    methods.push(ftest("test0N03", test0_n03));
    methods.push(ftest("test0N03r", test0_n03r));
    methods.push(ftest("test0W04", test0_w04));
    methods.push(ftest("test0N04", test0_n04));
    methods.push(ftest("test0N04r", test0_n04r));
    methods.push(ftest("test0W05", test0_w05));
    methods.push(ftest("test0W06", test0_w06));
    methods.push(ftest("test1Wtrunc01", test1_wtrunc01));
    methods.push(ftest("test1Wtrunc02", test1_wtrunc02));
    methods.push(ftest("test1Wtrunc02r", test1_wtrunc02r));
    methods.push(ftest("test1Ntrunc02", test1_ntrunc02));
    methods.push(ftest("test1Ntrunc02r", test1_ntrunc02r));
    methods.push(ftest("test1Wtrunc03", test1_wtrunc03));
    methods.push(ftest("test1Ntrunc03", test1_ntrunc03));
    methods.push(ftest("test1Wtrunc04", test1_wtrunc04));
    methods.push(ftest("test1Ntrunc04", test1_ntrunc04));
    methods.push(ftest("test1Wtrunc05", test1_wtrunc05));
    methods.push(ftest("test1Ntrunc05", test1_ntrunc05));
    methods.push(ftest("test1Wtrunc06", test1_wtrunc06));
    methods.push(ftest("test1Ntrunc06", test1_ntrunc06));
    methods.push(ftest("test1Ntrunc06r", test1_ntrunc06r));
    methods.push(ftest("test1Wtrunc07", test1_wtrunc07));
    methods.push(ftest("test1Wtrunc08", test1_wtrunc08));
    methods.push(ftest("test1Wtrunc09", test1_wtrunc09));
    methods.push(ftest("test1Ntrunc09", test1_ntrunc09));
    methods.push(ftest("test2Wprefix01", test2_wprefix01));
    methods.push(ftest("test2Nprefix01a", test2_nprefix01a));
    methods.push(ftest("test2Nprefix01b", test2_nprefix01b));
    methods.push(ftest("test2Wprefix02", test2_wprefix02));
    methods.push(ftest("test2Nprefix02a", test2_nprefix02a));
    methods.push(ftest("test2Nprefix02b", test2_nprefix02b));
    methods.push(ftest("test2Wnested01", test2_wnested01));
    methods.push(ftest("test2Nnested01", test2_nnested01));
    methods.push(ftest("test2Wnested02", test2_wnested02));
    methods.push(ftest("test2Nnested02", test2_nnested02));
    methods.push(ftest("test3Example01", test3_example01));
    fixture_class::<Test03Distance>(
        "org.apache.lucene.queryparser.surround.query.Test03Distance",
        methods,
    )
}

fn test00_exceptions(_t: &mut Test03Distance) -> Result<()> {
    let m = ExceptionQueryTestFacade::get_fail_queries(&EXCEPTION_QUERIES, VERBOSE);
    if !m.is_empty() {
        Assert::fail(&format!("No ParseException for:\n{m}"));
    }
    Ok(())
}

fn set_up_impl(t: &mut Test03Distance) -> Result<()> {
    t.db1 = Some(SingleFieldTestDb::new(&*random(), &DOCS1, FIELD_NAME));
    t.db2 = Some(SingleFieldTestDb::new(&*random(), &DOCS2, FIELD_NAME));
    t.db3 = Some(SingleFieldTestDb::new(&*random(), &DOCS3, FIELD_NAME));
    Ok(())
}

/// `distanceTst(String query, int[] expdnrs, SingleFieldTestDb db)`.
fn distance_tst(query: &str, expdnrs: &[i32], db: &SingleFieldTestDb) -> Result<()> {
    let mut tbq = BooleanQueryTestFacade::new(
        query,
        expdnrs,
        db,
        FIELD_NAME,
        BasicQueryFactory::with_max_basic_queries(MAX_BASIC_QUERIES),
    );
    tbq.set_verbose(VERBOSE);
    tbq.do_test()
}

/// `distanceTest1(String query, int[] expdnrs)`.
fn distance_test1(t: &Test03Distance, query: &str, expdnrs: &[i32]) -> Result<()> {
    distance_tst(query, expdnrs, t.db1.as_ref().expect("setUp ran"))
}

/// `distanceTest2(String query, int[] expdnrs)`.
fn distance_test2(t: &Test03Distance, query: &str, expdnrs: &[i32]) -> Result<()> {
    distance_tst(query, expdnrs, t.db2.as_ref().expect("setUp ran"))
}

/// `distanceTest3(String query, int[] expdnrs)`.
fn distance_test3(t: &Test03Distance, query: &str, expdnrs: &[i32]) -> Result<()> {
    distance_tst(query, expdnrs, t.db3.as_ref().expect("setUp ran"))
}

fn test0_w01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1 w word2", &expdnrs)
}

fn test0_n01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1 n word2", &expdnrs)
}

fn test0_w02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "word2 w word1", &expdnrs)
}

fn test0_w03(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "word2 2W word1", &expdnrs)
}

fn test0_n03(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word2 2N word1", &expdnrs)
}

fn test0_n03r(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1 2N word2", &expdnrs)
}

fn test0_w04(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "word2 3w word1", &expdnrs)
}

fn test0_n04(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word2 3n word1", &expdnrs)
}

fn test0_n04r(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1 3n word2", &expdnrs)
}

fn test0_w05(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "orda1 w orda3", &expdnrs)
}

fn test0_w06(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "orda1 2w orda3", &expdnrs)
}

fn test1_wtrunc01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1* w word2", &expdnrs)
}

fn test1_wtrunc02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word* w word2", &expdnrs)
}

fn test1_wtrunc02r(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 3];
    distance_test1(t, "word2 w word*", &expdnrs)
}

fn test1_ntrunc02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 3];
    distance_test1(t, "word* n word2", &expdnrs)
}

fn test1_ntrunc02r(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 3];
    distance_test1(t, "word2 n word*", &expdnrs)
}

fn test1_wtrunc03(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1* w word2*", &expdnrs)
}

fn test1_ntrunc03(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word1* N word2*", &expdnrs)
}

fn test1_wtrunc04(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "kxork* w kxor*", &expdnrs)
}

fn test1_ntrunc04(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "kxork* 99n kxor*", &expdnrs)
}

fn test1_wtrunc05(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 0] = [];
    distance_test1(t, "word2* 2W word1*", &expdnrs)
}

fn test1_ntrunc05(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word2* 2N word1*", &expdnrs)
}

fn test1_wtrunc06(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "ord* W word*", &expdnrs)
}

fn test1_ntrunc06(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "ord* N word*", &expdnrs)
}

fn test1_ntrunc06r(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "word* N ord*", &expdnrs)
}

fn test1_wtrunc07(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "(orda2 OR orda3) W word*", &expdnrs)
}

fn test1_wtrunc08(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "(orda2 OR orda3) W (word2 OR worda3)", &expdnrs)
}

fn test1_wtrunc09(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "(orda2 OR orda3) 2W (word2 OR worda3)", &expdnrs)
}

fn test1_ntrunc09(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [3];
    distance_test1(t, "(orda2 OR orda3) 2N (word2 OR worda3)", &expdnrs)
}

fn test2_wprefix01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test2(t, "W (w1, w2, w3)", &expdnrs)
}

fn test2_nprefix01a(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "N(w1, w2, w3)", &expdnrs)
}

fn test2_nprefix01b(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "N(w3, w1, w2)", &expdnrs)
}

fn test2_wprefix02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "2W(w1,w2,w3)", &expdnrs)
}

fn test2_nprefix02a(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "2N(w1,w2,w3)", &expdnrs)
}

fn test2_nprefix02b(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "2N(w2,w3,w1)", &expdnrs)
}

fn test2_wnested01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test2(t, "w1 W w2 W w3", &expdnrs)
}

fn test2_nnested01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 1] = [0];
    distance_test2(t, "w1 N w2 N w3", &expdnrs)
}

fn test2_wnested02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "w1 2W w2 2W w3", &expdnrs)
}

fn test2_nnested02(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 1];
    distance_test2(t, "w1 2N w2 2N w3", &expdnrs)
}

fn test3_example01(t: &mut Test03Distance) -> Result<()> {
    let expdnrs: [i32; 2] = [0, 2];
    distance_test3(t, "50n((low w pressure*) or depression*,5n(temperat*, (invers* or (negativ* 3n gradient*))),rain* or precipitat*)", &expdnrs)
}

fn test0_n01r(t: &mut Test03Distance) -> Result<()> {
    /* r reverse */
    let expdnrs: [i32; 1] = [0];
    distance_test1(t, "word2 n word1", &expdnrs)
}
