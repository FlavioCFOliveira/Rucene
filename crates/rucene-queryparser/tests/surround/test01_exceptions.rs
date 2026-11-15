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

//! Port of `Test01Exceptions.java` (its `main`, a JUnit 3 text runner, is
//! no test method).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;

use super::exception_query_test_facade::ExceptionQueryTestFacade;
use crate::qp_support::core_support::{class, test};
use crate::qp_support::jdk_default_hooks;

/// `boolean verbose = false`: to show actual parsing error messages.
const VERBOSE: bool = false;

/// `exceptionQueries`.
const EXCEPTION_QUERIES: [&str; 24] = [
    "*",
    "a*",
    "ab*",
    "?",
    "a?",
    "ab?",
    "a???b",
    "a?",
    "a*b?",
    "word1 word2",
    "word2 AND",
    "word1 OR",
    "AND(word2)",
    "AND(word2,)",
    "AND(word2,word1,)",
    "OR(word2)",
    "OR(word2 ,",
    "OR(word2 , word1 ,)",
    "xx NOT",
    "xx (a AND b)",
    "(a AND b",
    "a OR b)",
    "or(word2+ not ord+, and xyz,def)",
    "",
];

/// `Test01Exceptions`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(test("test01Exceptions", test01_exceptions));
    class(
        "org.apache.lucene.queryparser.surround.query.Test01Exceptions",
        methods,
    )
}

fn test01_exceptions() -> Result<()> {
    let m = ExceptionQueryTestFacade::get_fail_queries(&EXCEPTION_QUERIES, VERBOSE);
    if !m.is_empty() {
        Assert::fail(&format!("No ParseException for:\n{m}"));
    }
    Ok(())
}
