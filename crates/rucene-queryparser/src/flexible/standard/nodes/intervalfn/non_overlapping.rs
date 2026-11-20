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

//! Port of `NonOverlapping.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use super::interval_function::{require_non_null, IntervalFunction};

/// An interval function equivalent to `Intervals.nonOverlapping(IntervalsSource, IntervalsSource)`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.NonOverlapping`.
#[derive(Debug)]
pub struct NonOverlapping {
    minuend: Arc<dyn IntervalFunction>,
    subtrahend: Arc<dyn IntervalFunction>,
}

impl NonOverlapping {
    /// Equivalent to `new NonOverlapping(IntervalFunction minuend, IntervalFunction subtrahend)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `None`
    /// argument.
    pub fn new(
        minuend: Option<Arc<dyn IntervalFunction>>,
        subtrahend: Option<Arc<dyn IntervalFunction>>,
    ) -> Result<Self> {
        Ok(Self {
            minuend: require_non_null(minuend)?,
            subtrahend: require_non_null(subtrahend)?,
        })
    }
}

impl IntervalFunction for NonOverlapping {
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Ok(Intervals::non_overlapping(
            self.minuend.to_interval_source(field, analyzer)?,
            self.subtrahend.to_interval_source(field, analyzer)?,
        ))
    }

    /// Equivalent to `NonOverlapping.toString()`.
    fn to_java_string(&self) -> String {
        format!(
            "fn:nonOverlapping({} {})",
            self.minuend.to_java_string(),
            self.subtrahend.to_java_string()
        )
    }
}
