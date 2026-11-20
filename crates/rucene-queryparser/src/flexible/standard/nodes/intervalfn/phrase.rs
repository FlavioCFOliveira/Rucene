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

//! Port of `Phrase.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::{join, require_non_null, sources_of, IntervalFunction};

/// An interval function equivalent to `Intervals.phrase(IntervalsSource...)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.Phrase`.
#[derive(Debug)]
pub struct Phrase {
    sources: Vec<Arc<dyn IntervalFunction>>,
}

impl Phrase {
    /// Equivalent to `new Phrase(List<IntervalFunction> sources)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for `None`
    /// sources.
    pub fn new(sources: Option<Vec<Arc<dyn IntervalFunction>>>) -> Result<Self> {
        Ok(Self {
            sources: require_non_null(sources)?,
        })
    }
}

impl IntervalFunction for Phrase {
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Intervals::phrase(sources_of(&self.sources, field, analyzer)?)
    }

    /// Equivalent to `Phrase.toString()`.
    fn to_java_string(&self) -> String {
        format!("fn:phrase({})", join(&self.sources))
    }
}
