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

//! Port of `org.apache.lucene.tests.search.AssertingMatches`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::{Matches, MatchesIterator};

use crate::search::asserting_matches_iterator::AssertingMatchesIterator;

/// Wraps a [`Matches`] so that every [`MatchesIterator`] it hands out checks
/// its state machine.
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingMatches`.
pub struct AssertingMatches {
    /// `in`.
    inner: Arc<dyn Matches>,
}

impl AssertingMatches {
    /// `new AssertingMatches(matches)`.
    pub fn new(matches: Arc<dyn Matches>) -> AssertingMatches {
        AssertingMatches { inner: matches }
    }

    /// `AssertingMatches.unWrap(m)`: strips every `AssertingMatches` layer.
    pub fn un_wrap(m: Arc<dyn Matches>) -> Arc<dyn Matches> {
        let mut m = m;
        loop {
            let next = match m.as_any().downcast_ref::<AssertingMatches>() {
                Some(asserting) => Arc::clone(&asserting.inner),
                None => return m,
            };
            m = next;
        }
    }
}

impl Matches for AssertingMatches {
    fn get_matches(&self, field: &str) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(self
            .inner
            .get_matches(field)?
            .map(|mi| Box::new(AssertingMatchesIterator::new(mi)) as Box<dyn MatchesIterator>))
    }

    fn get_sub_matches(&self) -> Vec<Arc<dyn Matches>> {
        vec![Arc::clone(&self.inner)]
    }

    /// `iterator()`: the wrapped matches' fields.
    fn fields(&self) -> Vec<String> {
        self.inner.fields()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
