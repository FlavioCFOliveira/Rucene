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

//! Port of `org.apache.lucene.tests.search`.

pub mod asserting_accept_docs;
pub mod asserting_bulk_scorer;
pub mod asserting_collector;
pub mod asserting_index_searcher;
pub mod asserting_leaf_collector;
pub mod asserting_matches;
pub mod asserting_matches_iterator;
pub mod asserting_query;
pub mod asserting_scorable;
pub mod asserting_scorer;
pub mod asserting_weight;
pub mod base_explanation_test_case;
pub mod base_range_field_query_test_case;
pub mod block_score_query_wrapper;
pub mod bulk_scorer_wrapper_scorer;
pub mod check_hits;
pub mod disabling_bulk_scorer_query;
pub mod dummy_total_hit_count_collector;
pub mod fixed_bit_set_collector;
pub mod matches_test_base;
pub mod query_utils;
pub mod random_approximation_query;
pub mod scorer_index_searcher;
pub mod search_equivalence_test_base;
pub mod similarities;

pub use asserting_accept_docs::AssertingAcceptDocs;
pub use asserting_bulk_scorer::AssertingBulkScorer;
pub use asserting_collector::{AssertingCollector, AssertingCollectorState, CollectorRef};
pub use asserting_index_searcher::{AssertingIndexSearcher, AssertingIndexSearcherOverrides};
pub use asserting_leaf_collector::AssertingLeafCollector;
pub use asserting_matches::AssertingMatches;
pub use asserting_matches_iterator::AssertingMatchesIterator;
pub use asserting_query::AssertingQuery;
pub use asserting_scorable::{AssertingScorable, WrappedScorable};
pub use asserting_scorer::{AssertingScorer, IteratorState};
pub use asserting_weight::AssertingWeight;
pub use block_score_query_wrapper::BlockScoreQueryWrapper;
pub use bulk_scorer_wrapper_scorer::BulkScorerWrapperScorer;
pub use check_hits::{
    CheckHits, ExplanationAsserter, ExplanationAssertingSearcher, MatchesAsserter, SetCollector,
};
pub use disabling_bulk_scorer_query::DisablingBulkScorerQuery;
pub use dummy_total_hit_count_collector::{
    DummyTotalHitCountCollector, DummyTotalHitCountCollectorManager,
};
pub use fixed_bit_set_collector::{FixedBitSetCollector, FixedBitSetCollectorManager};
pub use query_utils::QueryUtils;
pub use random_approximation_query::{
    ApproximatedIterator, RandomApproximation, RandomApproximationQuery, RandomTwoPhaseView,
};
pub use scorer_index_searcher::{ScorerIndexSearcher, ScorerIndexSearcherOverrides};
