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

//! Port of `OffsetRange.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rucene::search::query::Query;

/// A non-empty range of offset positions.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.OffsetRange`.
///
/// Java's `MatchHighlighter.QueryOffsetRange` extends this class with the
/// query that produced the range and overrides `slice` to keep it; here a
/// range carries that query itself ([`query`](Self::query), `None` for a
/// plain `OffsetRange`), so lists of ranges keep their subclass instances as
/// Java's lists do. Equality and hashing read the offsets only, as Java's.
#[derive(Clone)]
pub struct OffsetRange {
    /// Start index, inclusive.
    pub from: i32,
    /// End index, exclusive.
    pub to: i32,
    query: Option<Arc<dyn Query>>,
}

impl OffsetRange {
    /// Equivalent to `new OffsetRange(int, int)`: `from` is the start
    /// offset, inclusive; `to` the end offset, exclusive.
    pub fn new(from: i32, to: i32) -> Self {
        debug_assert!(
            from <= to,
            "A non-empty offset range is required: {from}-{to}"
        );
        Self {
            from,
            to,
            query: None,
        }
    }

    /// `new QueryOffsetRange(query, from, to)` seen as an `OffsetRange`.
    pub(crate) fn with_query(query: Arc<dyn Query>, from: i32, to: i32) -> Self {
        Self {
            query: Some(query),
            ..Self::new(from, to)
        }
    }

    /// The query of a `QueryOffsetRange`: `Some` where Java's range is an
    /// `instanceof MatchHighlighter.QueryOffsetRange`.
    pub fn query(&self) -> Option<&Arc<dyn Query>> {
        self.query.as_ref()
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> i32 {
        self.to - self.from
    }

    /// Returns a sub slice of this range (a copy) with the given start and
    /// end offsets; a `QueryOffsetRange` keeps its query (its `slice`
    /// override).
    ///
    /// Equivalent to `slice(int, int)`.
    pub fn slice(&self, from: i32, to: i32) -> OffsetRange {
        debug_assert!(from >= self.from);
        debug_assert!(to <= self.to);
        OffsetRange {
            from,
            to,
            query: self.query.clone(),
        }
    }

    /// Returns `true` if this range contains or is equal to the other range.
    ///
    /// Equivalent to `contains(OffsetRange)`.
    pub fn contains(&self, other: &OffsetRange) -> bool {
        self.from <= other.from && self.to >= other.to
    }

    /// Equivalent to `hashCode()`: `Objects.hash(from, to)`.
    pub fn hash_code(&self) -> i32 {
        let mut result = 1i32;
        result = result.wrapping_mul(31).wrapping_add(self.from);
        result.wrapping_mul(31).wrapping_add(self.to)
    }
}

impl fmt::Display for OffsetRange {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[from={}, to={}]", self.from, self.to)
    }
}

impl fmt::Debug for OffsetRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl PartialEq for OffsetRange {
    /// Equivalent to `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.from == other.from && self.to == other.to
    }
}

impl Eq for OffsetRange {}

impl Hash for OffsetRange {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

/// The offsets of an `OffsetRange` or of one of its subclasses (`Passage`).
pub trait HasOffsetRange {
    /// The field `from`.
    fn range_from(&self) -> i32;
    /// The field `to`.
    fn range_to(&self) -> i32;
}

impl HasOffsetRange for OffsetRange {
    fn range_from(&self) -> i32 {
        self.from
    }

    fn range_to(&self) -> i32 {
        self.to
    }
}
