//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/TestDisjunctionScoreBlockBoundaryPropagator.java`.
//!
//! **Divergence from Lucene 10.5.0.** Rucene's propagator does not keep the
//! sub-scorers it is built with: it reads them through a `SubScorers` view
//! passed to `new` and to `advance_shallow`, here the shuffled list.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::disjunction_score_block_boundary_propagator::{
    DisjunctionScoreBlockBoundaryPropagator, SubScorers,
};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::scorable::Scorable;
use rucene::search::scorer::Scorer;

use crate::core_support::{assert_equals, class, random, test};

/// `TestDisjunctionScoreBlockBoundaryPropagator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDisjunctionScoreBlockBoundaryPropagator",
        vec![test("testBasics", test_basics)],
    )
}

fn uoe() -> ! {
    Throwable::new("java.lang.UnsupportedOperationException", "").throw()
}

/// `FakeScorer extends Scorer`.
#[derive(Debug)]
struct FakeScorer {
    boundary: i32,
    max_score: f32,
}

impl Scorable for FakeScorer {
    fn score(&mut self) -> Result<f32> {
        uoe()
    }

    fn set_min_competitive_score(&mut self, _min_competitive_score: f32) -> Result<()> {
        Ok(())
    }
}

impl Scorer for FakeScorer {
    fn doc_id(&self) -> i32 {
        0
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        uoe()
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.max_score)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        // Java `assert`, enabled under the test runner.
        Assert::assert_true(target <= self.boundary);
        Ok(self.boundary)
    }
}

/// `List<Scorer>` seen as the propagator's sub-scorers.
struct ScorerList(Vec<FakeScorer>);

impl SubScorers for ScorerList {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn sub_scorer(&mut self, index: usize) -> &mut dyn Scorer {
        &mut self.0[index]
    }
}

fn test_basics() -> Result<()> {
    let scorer1 = FakeScorer {
        boundary: 20,
        max_score: 0.5,
    };
    let scorer2 = FakeScorer {
        boundary: 50,
        max_score: 1.5,
    };
    let scorer3 = FakeScorer {
        boundary: 30,
        max_score: 2.0,
    };
    let scorer4 = FakeScorer {
        boundary: 80,
        max_score: 3.0,
    };
    let mut scorers = vec![scorer1, scorer2, scorer3, scorer4];
    shuffle(&mut scorers, &*random());
    let mut scorers = ScorerList(scorers);
    let mut propagator = DisjunctionScoreBlockBoundaryPropagator::new(&mut scorers)?;
    assert_equals(&20, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(0.2);
    assert_equals(&20, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(0.7);
    assert_equals(&30, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(1.2);
    assert_equals(&30, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(1.7);
    assert_equals(&30, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(2.2);
    assert_equals(&80, &propagator.advance_shallow(&mut scorers, 0)?);

    propagator.set_min_competitive_score(5.0);
    assert_equals(&80, &propagator.advance_shallow(&mut scorers, 0)?);
    Ok(())
}
