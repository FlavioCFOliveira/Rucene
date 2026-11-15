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

//! Port of `MaxWidth.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::{require_non_null, IntervalFunction};

/// An interval function equivalent to `Intervals.maxwidth(int, IntervalsSource)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.MaxWidth`.
#[derive(Debug)]
pub struct MaxWidth {
    width: i32,
    source: Arc<dyn IntervalFunction>,
}

impl MaxWidth {
    /// Equivalent to `new MaxWidth(int width, IntervalFunction source)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `None`
    /// source.
    pub fn new(width: i32, source: Option<Arc<dyn IntervalFunction>>) -> Result<Self> {
        Ok(Self {
            width,
            source: require_non_null(source)?,
        })
    }
}

impl IntervalFunction for MaxWidth {
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Ok(Intervals::maxwidth(
            self.width,
            self.source.to_interval_source(field, analyzer)?,
        ))
    }

    fn to_java_string(&self) -> String {
        format!(
            "fn:maxwidth({} {})",
            self.width,
            self.source.to_java_string()
        )
    }
}
