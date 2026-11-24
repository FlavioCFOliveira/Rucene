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

//! Shared plumbing for the queries of the crate: a scorer shared between
//! the `DocIdSetIterator` of a `TwoPhaseIterator` and the `DoubleValues`
//! Java's `DoubleValuesSource.fromScorer(scorer)` reads from it.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::double_values::DoubleValues;
use rucene::search::Scorer;

/// A scorer that two readers share, as Java shares the `Scorer` object
/// between `scorer.iterator()` and `DoubleValuesSource.fromScorer(scorer)`.
pub(crate) type SharedScorer = Rc<RefCell<Box<dyn Scorer>>>;

/// `scorer.iterator()` over a [`SharedScorer`].
pub(crate) struct ScorerDisi(pub(crate) SharedScorer);

impl DocIdSetIterator for ScorerDisi {
    fn doc_id(&self) -> i32 {
        self.0.borrow_mut().iterator().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.0.borrow_mut().iterator().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.0.borrow_mut().iterator().advance(target)
    }

    fn cost(&self) -> i64 {
        self.0.borrow_mut().iterator().cost()
    }
}

/// `DoubleValuesSource.fromScorer(scorer)` over a [`SharedScorer`]: the
/// score of the current document; `advanceExact` answers `true`.
pub(crate) struct ScorerScores(pub(crate) SharedScorer);

impl DoubleValues for ScorerScores {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.0.borrow_mut().score()?))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

/// Shares `scorer`.
pub(crate) fn share(scorer: Box<dyn Scorer>) -> SharedScorer {
    Rc::new(RefCell::new(scorer))
}
