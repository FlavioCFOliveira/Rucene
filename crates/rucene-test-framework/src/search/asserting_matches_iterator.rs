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

//! Port of `org.apache.lucene.tests.search.AssertingMatchesIterator`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::{MatchesIterator, Query};

/// `AssertingMatchesIterator.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Unpositioned,
    Iterating,
    Exhausted,
}

impl fmt::Display for State {
    /// The Java constant name, as `assert …: state` prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::Unpositioned => "UNPOSITIONED",
            State::Iterating => "ITERATING",
            State::Exhausted => "EXHAUSTED",
        })
    }
}

/// Wraps a [`MatchesIterator`] and checks that it is not read before `next()`
/// returned `true`, nor advanced once exhausted.
///
/// Equivalent to the package-private
/// `org.apache.lucene.tests.search.AssertingMatchesIterator`. The accessors
/// that cannot fail in Rust (`start_position`, `end_position`, `get_query`)
/// fail a check by panicking with Java's `AssertionError`.
pub struct AssertingMatchesIterator {
    /// `in`.
    inner: Box<dyn MatchesIterator>,
    /// `state`.
    state: State,
}

impl AssertingMatchesIterator {
    /// `new AssertingMatchesIterator(in)`.
    pub fn new(inner: Box<dyn MatchesIterator>) -> AssertingMatchesIterator {
        AssertingMatchesIterator {
            inner,
            state: State::Unpositioned,
        }
    }
}

impl MatchesIterator for AssertingMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        java_assert!(self.state != State::Exhausted, "{}", self.state);
        let more = self.inner.next()?;
        self.state = if more {
            State::Iterating
        } else {
            State::Exhausted
        };
        Ok(more)
    }

    fn start_position(&self) -> i32 {
        java_assert_unchecked!(self.state == State::Iterating, "{}", self.state);
        self.inner.start_position()
    }

    fn end_position(&self) -> i32 {
        java_assert_unchecked!(self.state == State::Iterating, "{}", self.state);
        self.inner.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        java_assert!(self.state == State::Iterating, "{}", self.state);
        self.inner.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        java_assert!(self.state == State::Iterating, "{}", self.state);
        self.inner.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        java_assert!(self.state == State::Iterating, "{}", self.state);
        self.inner.get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        java_assert_unchecked!(self.state == State::Iterating, "{}", self.state);
        self.inner.get_query()
    }
}
