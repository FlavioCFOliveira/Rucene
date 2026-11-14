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

//! Port of `SpanCollector.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::index::{PostingsEnum, Term};

/// The collection of postings information from the leaves of a
/// [`Spans`](crate::spans::spans::Spans).
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanCollector`.
pub trait SpanCollector {
    /// Collects information from postings.
    ///
    /// Equivalent to `SpanCollector.collectLeaf(PostingsEnum, int, Term)`.
    ///
    /// * `postings` — a [`PostingsEnum`];
    /// * `position` — the position of the postings enum;
    /// * `term` — the [`Term`] for this postings list.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the postings.
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        position: i32,
        term: &Term,
    ) -> Result<()>;

    /// Called to indicate that the driving spans has moved to a new position.
    ///
    /// Equivalent to `SpanCollector.reset()`.
    fn reset(&mut self);
}
