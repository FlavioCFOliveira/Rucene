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

//! Port of `IntervalFunction.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_queries::intervals::intervals_source::IntervalsSource;

/// Representation of an interval function that can be converted to
/// `IntervalsSource`.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.IntervalFunction`;
/// its abstract `toString()` is [`to_java_string`](IntervalFunction::to_java_string).
pub trait IntervalFunction: fmt::Debug + Send + Sync {
    /// Equivalent to `toIntervalSource(String field, Analyzer analyzer)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of building the source.
    fn to_interval_source(
        &self,
        field: &str,
        analyzer: &dyn Analyzer,
    ) -> Result<Arc<dyn IntervalsSource>>;

    /// Equivalent to `toString()`.
    fn to_java_string(&self) -> String;
}

/// `sources.stream().map(f -> f.toIntervalSource(field, analyzer))`.
pub(crate) fn sources_of(
    sources: &[Arc<dyn IntervalFunction>],
    field: &str,
    analyzer: &dyn Analyzer,
) -> Result<Vec<Arc<dyn IntervalsSource>>> {
    sources
        .iter()
        .map(|f| f.to_interval_source(field, analyzer))
        .collect()
}

/// `sources.stream().map(IntervalFunction::toString).collect(Collectors.joining(" "))`.
pub(crate) fn join(sources: &[Arc<dyn IntervalFunction>]) -> String {
    sources
        .iter()
        .map(|f| f.to_java_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `NullPointerException` of `Objects.requireNonNull`.
pub(crate) fn require_non_null<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(rucene::error::LuceneError::null_pointer)
}
