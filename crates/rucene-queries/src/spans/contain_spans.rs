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

//! Port of `ContainSpans.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;

use crate::spans::conjunction_spans::ConjunctionSpans;
use crate::spans::span_collector::SpanCollector;
use crate::spans::spans::{SharedSpans, Spans, NO_MORE_POSITIONS};

/// The common state of the spans of
/// [`SpanContainingQuery`](crate::spans::span_containing_query::SpanContainingQuery)
/// and [`SpanWithinQuery`](crate::spans::span_within_query::SpanWithinQuery).
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.queries.spans.ContainSpans`, which extends
/// `ConjunctionSpans`. Its two anonymous subclasses hold one and add the
/// abstract `twoPhaseCurrentDocMatches()` and `nextStartPosition()`.
pub(crate) struct ContainSpans {
    pub(crate) conj: ConjunctionSpans,
    /// The `sourceSpans` field: one of `bigSpans` and `littleSpans`.
    pub(crate) source_spans: SharedSpans,
    /// The `bigSpans` field.
    pub(crate) big_spans: SharedSpans,
    /// The `littleSpans` field.
    pub(crate) little_spans: SharedSpans,
}

impl ContainSpans {
    /// Equivalent to the package-private `ContainSpans(Spans, Spans, Spans)`,
    /// whose third argument is always its first or its second: `source_is_big`
    /// says which.
    ///
    /// # Errors
    ///
    /// Propagates the error the conjunction of the two spans raises.
    pub(crate) fn new(
        big_spans: Box<dyn Spans>,
        little_spans: Box<dyn Spans>,
        source_is_big: bool,
    ) -> Result<Self> {
        let conj = ConjunctionSpans::new(vec![big_spans, little_spans])?;
        let big_spans = conj.sub_spans[0].clone();
        let little_spans = conj.sub_spans[1].clone();
        let source_spans = if source_is_big {
            big_spans.clone()
        } else {
            little_spans.clone()
        };
        Ok(Self {
            conj,
            source_spans,
            big_spans,
            little_spans,
        })
    }

    /// `ContainSpans.startPosition()`.
    pub(crate) fn start_position(&self) -> i32 {
        if self.conj.at_first_in_current_doc {
            -1
        } else if self.conj.one_exhausted_in_current_doc {
            NO_MORE_POSITIONS
        } else {
            self.source_spans.start_position()
        }
    }

    /// `ContainSpans.endPosition()`.
    pub(crate) fn end_position(&self) -> i32 {
        if self.conj.at_first_in_current_doc {
            -1
        } else if self.conj.one_exhausted_in_current_doc {
            NO_MORE_POSITIONS
        } else {
            self.source_spans.end_position()
        }
    }

    /// `ContainSpans.width()`.
    pub(crate) fn width(&self) -> i32 {
        self.source_spans.width()
    }

    /// `ContainSpans.collect(SpanCollector)`.
    pub(crate) fn collect(&self, collector: &mut dyn SpanCollector) -> Result<()> {
        self.big_spans.collect(collector)?;
        self.little_spans.collect(collector)
    }
}
