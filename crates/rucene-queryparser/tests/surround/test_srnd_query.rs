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

//! Port of `TestSrndQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene_queryparser::surround::parser::query_parser::QueryParser;
use rucene_queryparser::surround::query::basic_query_factory::BasicQueryFactory;
use rucene_test_framework::search::query_utils::QueryUtils;

use crate::qp_support::core_support::{class, test};
use crate::qp_support::jdk_default_hooks;

/// `TestSrndQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    // `@Test public void testHashEquals()`.
    methods.push(test("testHashEquals", test_hash_equals));
    class(
        "org.apache.lucene.queryparser.surround.query.TestSrndQuery",
        methods,
    )
}

/// `checkEqualParsings(String s1, String s2)`.
fn check_equal_parsings(s1: &str, s2: &str) -> Result<()> {
    let field_name = "foo";
    let qf = Arc::new(BasicQueryFactory::with_max_basic_queries(16));
    let lq1 = QueryParser::parse(s1)?.make_lucene_query_field(Some(field_name), &qf)?;
    let lq2 = QueryParser::parse(s2)?.make_lucene_query_field(Some(field_name), &qf)?;
    QueryUtils::check_equal(&*lq1, &*lq2);
    Ok(())
}

fn test_hash_equals() -> Result<()> {
    // grab some sample queries from Test02Boolean and Test03Distance and
    // check there hashes and equals
    check_equal_parsings("word1 w word2", " word1  w  word2 ")?;
    check_equal_parsings("2N(w1,w2,w3)", " 2N(w1, w2 , w3)")?;
    check_equal_parsings("abc?", " abc? ")?;
    check_equal_parsings("w*rd?", " w*rd?")?;
    Ok(())
}
