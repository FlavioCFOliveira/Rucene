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

//! Port of `ExceptionQueryTestFacade.java`.

use rucene_queryparser::surround::parser::query_parser::QueryParser;

use crate::qp_support::{catch_java, SURROUND_PARSE_EXCEPTION};

/// `public class ExceptionQueryTestFacade`.
pub struct ExceptionQueryTestFacade {
    query_text: String,
    verbose: bool,
}

impl ExceptionQueryTestFacade {
    /// `new ExceptionQueryTestFacade(String queryText, boolean verbose)`.
    pub fn new(query_text: &str, verbose: bool) -> Self {
        ExceptionQueryTestFacade {
            query_text: query_text.to_string(),
            verbose,
        }
    }

    /// `doTest(StringBuilder failQueries)`: only a `ParseException` is
    /// caught; any other throwable propagates.
    pub fn do_test(&self, fail_queries: &mut String) {
        let mut pass = false;
        let mut lq = None;
        match catch_java(|| QueryParser::parse(&self.query_text)) {
            Ok(q) => {
                if self.verbose {
                    println!(
                        "Query: {}\nParsed as: {}",
                        self.query_text,
                        q.srnd_to_string()
                    );
                }
                lq = Some(q);
            }
            Err(e) if e.is_instance(SURROUND_PARSE_EXCEPTION) => {
                if self.verbose {
                    println!(
                        "Parse exception for query:\n{}\n{}",
                        self.query_text,
                        e.message().unwrap_or("null")
                    );
                }
                pass = true;
            }
            Err(other) => other.throw(),
        }
        if !pass {
            fail_queries.push_str(&self.query_text);
            fail_queries.push_str("\nParsed as: ");
            fail_queries.push_str(&lq.map_or_else(|| "null".to_string(), |q| q.srnd_to_string()));
            fail_queries.push('\n');
        }
    }

    /// `getFailQueries(String[] exceptionQueries, boolean verbose)`.
    pub fn get_fail_queries(exception_queries: &[&str], verbose: bool) -> String {
        let mut fail_queries = String::new();
        for q in exception_queries {
            ExceptionQueryTestFacade::new(q, verbose).do_test(&mut fail_queries);
        }
        fail_queries
    }
}
