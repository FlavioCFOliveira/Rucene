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

//! Port of `Extend.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::{require_non_null, IntervalFunction};

/// An interval function equivalent to `Intervals.extend(IntervalsSource,
/// int, int)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.Extend`.
#[derive(Debug)]
pub struct Extend {
    before: i32,
    after: i32,
    source: Arc<dyn IntervalFunction>,
}

impl Extend {
    /// Equivalent to `new Extend(IntervalFunction source, int before, int
    /// after)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `None`
    /// source.
    pub fn new(source: Option<Arc<dyn IntervalFunction>>, before: i32, after: i32) -> Result<Self> {
        Ok(Self {
            source: require_non_null(source)?,
            before,
            after,
        })
    }
}

impl IntervalFunction for Extend {
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Ok(Intervals::extend(
            self.source.to_interval_source(field, analyzer)?,
            self.before,
            self.after,
        ))
    }

    /// Equivalent to `Extend.toString()`.
    fn to_java_string(&self) -> String {
        format!(
            "fn:extend({} {} {})",
            self.source.to_java_string(),
            self.before,
            self.after
        )
    }
}
