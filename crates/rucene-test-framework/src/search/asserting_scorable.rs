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

//! Port of `org.apache.lucene.tests.search.AssertingScorable`.

use rucene::error::Result;
use rucene::search::{ChildScorable, Scorable};
use rucene::util::java_fmt::float_to_string;

/// Wraps a [`Scorable`] to make sure its scores are not negative (nor NaN).
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingScorable`, a
/// `FilterScorable`. It wraps a borrow because this port passes the scorable to
/// every collection call instead of letting the collector keep it (see
/// [`rucene::search::LeafCollector`]).
///
/// **Divergences.**
///
/// * Java's `wrap` returns its argument when it already is an
///   `AssertingScorable` or an `AssertingScorer`, and wraps a `Scorer` with
///   `AssertingScorer.wrap(scorer, true, true)` instead. A `&mut dyn Scorable`
///   can be tested for neither in this port, so [`wrap`](Self::wrap) always
///   builds this wrapper: a second wrapper repeats the same check, and the
///   `Scorer` branch's checks (the scorer is positioned, the score is not NaN
///   and below the max score after `advanceShallow`, and is not read during a
///   batch collection) are not made.
/// * The failure message ends with `in=<scorable>`: Rust's [`Scorable`] has no
///   string form.
pub struct AssertingScorable<'a> {
    inner: &'a mut dyn Scorable,
}

impl<'a> AssertingScorable<'a> {
    /// `new AssertingScorable(in)`.
    pub fn new(inner: &'a mut dyn Scorable) -> AssertingScorable<'a> {
        AssertingScorable { inner }
    }

    /// `AssertingScorable.wrap(in)` (see the divergences).
    pub fn wrap(inner: &'a mut dyn Scorable) -> AssertingScorable<'a> {
        AssertingScorable::new(inner)
    }
}

impl Scorable for AssertingScorable<'_> {
    fn score(&mut self) -> Result<f32> {
        let score = self.inner.score()?;
        // Note: score >= 0 returns false for NaN
        let non_negative = score >= 0.0;
        java_assert!(
            non_negative,
            "score={} for in=<scorable>",
            float_to_string(score)
        );
        Ok(score)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }

    /// `FilterScorable.getChildren()`.
    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![ChildScorable::new(&mut *self.inner, "FILTER")])
    }
}
