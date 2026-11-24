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

//! Port of `OffsetsRetrievalStrategy.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use rucene::error::Result;
use rucene::search::matches::MatchesIterator;

use crate::matchhighlight::match_region_retriever::FieldValueProvider;
use crate::matchhighlight::offset_range::OffsetRange;

/// Determines how match offset regions are computed from `MatchesIterator`.
/// Several possibilities exist, ranging from retrieving offsets directly
/// from a match instance to re-evaluating the document's field and
/// recomputing offsets from there.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetsRetrievalStrategy`.
pub trait OffsetsRetrievalStrategy: Send + Sync {
    /// Return value offsets (match ranges) acquired from the given
    /// `MatchesIterator`.
    ///
    /// Equivalent to `get(MatchesIterator, FieldValueProvider)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn get(
        &self,
        matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>>;
}

/// A strategy written as a closure (Java's lambdas).
impl<F> OffsetsRetrievalStrategy for F
where
    F: Fn(&mut dyn MatchesIterator, &dyn FieldValueProvider) -> Result<Vec<OffsetRange>>
        + Send
        + Sync,
{
    fn get(
        &self,
        matches_iterator: &mut dyn MatchesIterator,
        doc: &dyn FieldValueProvider,
    ) -> Result<Vec<OffsetRange>> {
        self(matches_iterator, doc)
    }
}
