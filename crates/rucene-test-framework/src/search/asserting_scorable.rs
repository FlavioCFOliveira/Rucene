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

use std::any::TypeId;

use rucene::error::Result;
use rucene::search::{ChildScorable, Scorable, Scorer};
use rucene::util::java_fmt::float_to_string;

use crate::search::asserting_scorer::{AssertingScorer, AssertingScorerState};

/// Wraps a [`Scorable`] to make sure its scores are not negative (nor NaN).
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingScorable`, a
/// `FilterScorable`. It wraps a borrow because this port passes the scorable to
/// every collection call instead of letting the collector keep it (see
/// [`rucene::search::LeafCollector`]).
///
/// The `instanceof` tests of [`wrap`](Self::wrap) read
/// [`Scorable::class_id`] and [`Scorable::as_scorer`].
///
/// **Divergence.** The failure message ends with `in=<scorable>`: Rust's
/// [`Scorable`] has no string form.
pub struct AssertingScorable<'a> {
    inner: &'a mut dyn Scorable,
}

/// What [`AssertingScorable::wrap`] returns: Java's `Scorable`, which is the
/// argument itself, a new `AssertingScorer`, or a new `AssertingScorable`.
pub enum WrappedScorable<'a> {
    /// The argument, already an `AssertingScorable` or an `AssertingScorer`.
    Unchanged(&'a mut dyn Scorable),
    /// `AssertingScorer.wrap(scorer, true, true)` over a `Scorer` argument.
    Scorer(AssertingScorer<'a>),
    /// `new AssertingScorable(in)`.
    Scorable(AssertingScorable<'a>),
}

impl WrappedScorable<'_> {
    /// The wrapped scorable as a [`Scorer`] (`wrappedScorer instanceof
    /// Scorer`).
    pub fn as_scorer(&mut self) -> Option<&mut dyn Scorer> {
        match self {
            WrappedScorable::Unchanged(scorable) => scorable.as_scorer(),
            WrappedScorable::Scorer(scorer) => Some(scorer),
            WrappedScorable::Scorable(_) => None,
        }
    }

    /// The wrapped scorable as a [`Scorable`].
    pub fn as_scorable(&mut self) -> &mut dyn Scorable {
        match self {
            WrappedScorable::Unchanged(scorable) => &mut **scorable,
            WrappedScorable::Scorer(scorer) => scorer,
            WrappedScorable::Scorable(scorable) => scorable,
        }
    }

    /// The state of the new `AssertingScorer`, if one was built.
    pub(crate) fn scorer_state(&self) -> Option<AssertingScorerState> {
        match self {
            WrappedScorable::Scorer(scorer) => Some(scorer.state_snapshot()),
            _ => None,
        }
    }
}

impl<'a> AssertingScorable<'a> {
    /// `new AssertingScorable(in)`.
    pub fn new(inner: &'a mut dyn Scorable) -> AssertingScorable<'a> {
        AssertingScorable { inner }
    }

    /// `AssertingScorable.wrap(in)`.
    pub fn wrap(inner: &'a mut dyn Scorable) -> WrappedScorable<'a> {
        Self::wrap_with_state(inner, None)
    }

    /// [`wrap`](Self::wrap), whose new `AssertingScorer` (if any) resumes the
    /// state machine of an earlier wrapper of the same scorer (see
    /// [`AssertingScorer`]'s `wrap_borrowed`).
    pub(crate) fn wrap_with_state(
        inner: &'a mut dyn Scorable,
        state: Option<AssertingScorerState>,
    ) -> WrappedScorable<'a> {
        let class = inner.class_id();
        if class == Some(TypeId::of::<AssertingScorable<'static>>())
            || class == Some(TypeId::of::<AssertingScorer<'static>>())
        {
            return WrappedScorable::Unchanged(inner);
        }

        // If `in` is Scorer, we need to wrap it as a Scorer instead of Scorable because
        // NumericComparator uses the iterator cost of a Scorer in sort optimization.
        if inner.as_scorer().is_some() {
            let scorer = inner
                .as_scorer()
                .expect("INVARIANT: as_scorer answered Some on the line above");
            WrappedScorable::Scorer(AssertingScorer::wrap_borrowed(scorer, state))
        } else {
            WrappedScorable::Scorable(AssertingScorable::new(inner))
        }
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

    /// `instanceof AssertingScorable` (see [`Scorable::class_id`]).
    fn class_id(&self) -> Option<TypeId> {
        Some(TypeId::of::<AssertingScorable<'static>>())
    }
}
