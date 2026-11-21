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

//! Port of `Wildcard.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::util::BytesRef;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::IntervalFunction;

/// Node that represents `Intervals.wildcard(BytesRef)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.Wildcard`.
#[derive(Debug)]
pub struct Wildcard {
    wildcard: String,
    max_expansions: i32,
}

impl Wildcard {
    /// Equivalent to `new Wildcard(String wildcard, int maxExpansions)`.
    pub fn new(wildcard: impl Into<String>, max_expansions: i32) -> Self {
        Self {
            wildcard: wildcard.into(),
            max_expansions,
        }
    }
}

impl IntervalFunction for Wildcard {
    fn to_interval_source(
        &self,
        _field: &str,
        _analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let wildcard = BytesRef::from_char_sequence(&self.wildcard);
        if self.max_expansions == 0 {
            Intervals::wildcard(&wildcard)
        } else {
            Intervals::wildcard_with_max_expansions(&wildcard, self.max_expansions)
        }
    }

    /// Equivalent to `Wildcard.toString()`.
    fn to_java_string(&self) -> String {
        format!(
            "fn:wildcard({}{})",
            self.wildcard,
            if self.max_expansions == 0 {
                String::new()
            } else {
                format!(" maxExpansions:{}", self.max_expansions)
            }
        )
    }
}
