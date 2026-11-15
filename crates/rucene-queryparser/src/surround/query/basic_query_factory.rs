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

//! The port of `BasicQueryFactory.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::{Mutex, PoisonError};

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::TermQuery;
use rucene::util::java_collections::java_string_hash_code;
use rucene_queries::spans::span_term_query::SpanTermQuery;

use super::too_many_basic_queries::TooManyBasicQueries;

/// The Java binary class name.
const CLASS: &str = "org.apache.lucene.queryparser.surround.query.BasicQueryFactory";

/// Factory for creating basic term queries.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.BasicQueryFactory`;
/// the rewrite queries share one factory, whose count of queries made is
/// behind a lock, as Java's `synchronized checkMax()`.
///
/// **Divergence.** Java's `hashCode()` mixes `getClass().hashCode()`, an
/// identity hash that differs between JVM runs; this port mixes the
/// `String.hashCode()` of the class name instead.
#[derive(Debug)]
pub struct BasicQueryFactory {
    max_basic_queries: i32,
    queries_made: Mutex<i32>,
}

impl BasicQueryFactory {
    /// Equivalent to `new BasicQueryFactory(int)`.
    pub fn with_max_basic_queries(max_basic_queries: i32) -> Self {
        Self {
            max_basic_queries,
            queries_made: Mutex::new(0),
        }
    }

    /// Equivalent to `new BasicQueryFactory()`, a maximum of 1024 basic
    /// queries.
    pub fn new() -> Self {
        Self::with_max_basic_queries(1024)
    }

    fn made(&self) -> i32 {
        *self
            .queries_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Equivalent to `getNrQueriesMade()`.
    pub fn get_nr_queries_made(&self) -> i32 {
        self.made()
    }

    /// Equivalent to `getMaxBasicQueries()`.
    pub fn get_max_basic_queries(&self) -> i32 {
        self.max_basic_queries
    }

    /// Equivalent to `toString()`.
    pub fn java_to_string(&self) -> String {
        format!(
            "{CLASS}(maxBasicQueries: {}, queriesMade: {})",
            self.max_basic_queries,
            self.made()
        )
    }

    /// Equivalent to the private `atMax()`.
    fn at_max(&self) -> bool {
        self.made() >= self.max_basic_queries
    }

    /// Equivalent to the protected `synchronized checkMax()`.
    ///
    /// # Errors
    ///
    /// [`TooManyBasicQueries`] once the maximum is reached.
    pub fn check_max(&self) -> Result<()> {
        let mut made = self
            .queries_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *made >= self.max_basic_queries {
            return Err(TooManyBasicQueries::new(self.max_basic_queries).error());
        }
        *made += 1;
        Ok(())
    }

    /// Equivalent to `newTermQuery(Term)`.
    ///
    /// # Errors
    ///
    /// As [`check_max`](Self::check_max).
    pub fn new_term_query(&self, term: Term) -> Result<TermQuery> {
        self.check_max()?;
        Ok(TermQuery::new(term))
    }

    /// Equivalent to `newSpanTermQuery(Term)`.
    ///
    /// # Errors
    ///
    /// As [`check_max`](Self::check_max).
    pub fn new_span_term_query(&self, term: Term) -> Result<SpanTermQuery> {
        self.check_max()?;
        Ok(SpanTermQuery::new(term))
    }

    /// Equivalent to `hashCode()`: `getClass().hashCode() ^ (atMax() ? 7 :
    /// 31 * 32)`.
    pub fn java_hash_code(&self) -> i32 {
        java_string_hash_code(CLASS) ^ (if self.at_max() { 7 } else { 31 * 32 })
    }

    /// Two BasicQueryFactory's are equal when they generate the same types
    /// of basic queries, or both cannot generate queries anymore.
    ///
    /// Equivalent to `equals(Object)`.
    pub fn java_equals(&self, other: &BasicQueryFactory) -> bool {
        self.at_max() == other.at_max()
    }
}

impl Default for BasicQueryFactory {
    fn default() -> Self {
        Self::new()
    }
}
