//! Port of the test classes of `lucene/core/src/test/org/apache/lucene/search/`
//! and its `comparators`, `knn` and `similarities` packages (sprint 9, tasks
//! #389 to #397). Each class runs as a suite under `LuceneTestCase`'s rule
//! chain.

// The ports keep Java's indexed loops, counters and unit bindings line by
// line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

mod core_search {
    pub mod support;

    pub mod similarities;
    pub mod test_accept_docs;
    pub mod test_approximation_search_equivalence;
    pub mod test_automaton_query;
    pub mod test_automaton_query_unicode;
    pub mod test_base_range_filter;
    pub mod test_bayesian_score_query;
    pub mod test_binary_sort_field;
    pub mod test_bit_set_doc_id_stream;
    pub mod test_blended_term_query;
    pub mod test_block_max_conjunction;
    pub mod test_boolean2;
    pub mod test_boolean_min_should_match;
    pub mod test_boolean_or;
    pub mod test_boost_query;
    pub mod test_caching_collector;
    pub mod test_collector_manager;
    pub mod test_conjunction_disi;
    pub mod test_conjunctions;
    pub mod test_constant_score_bulk_scorer;
    pub mod test_constant_score_query;
    pub mod test_constant_score_scorer;
    pub mod test_custom_searcher_sort;
    pub mod test_date_sort;
    pub mod test_multi_leaf_knn_collector;
    pub mod test_numeric_comparator;
    pub mod test_term_ord_val_comparator_adaptive_skipping;
    pub mod test_updateable_doc_id_set_iterator;
}

use core_search::*;
use core_support::run;

#[test]
fn test_axiomatic_similarity() {
    run(similarities::axiomatic_class_def);
}

#[test]
fn test_bm25_similarity() {
    run(similarities::bm25_class_def);
}

#[test]
fn test_boolean_similarity() {
    run(similarities::boolean_class_def);
}

#[test]
fn test_classic_similarity() {
    run(similarities::classic_class_def);
}

#[test]
fn test_raw_tf_similarity() {
    run(similarities::raw_tf_class_def);
}

#[test]
fn test_similarity2() {
    run(similarities::similarity2_class_def);
}

#[test]
fn test_similarity_base() {
    run(similarities::similarity_base_class_def);
}

#[test]
fn test_updateable_doc_id_set_iterator() {
    run(test_updateable_doc_id_set_iterator::class_def);
}

#[test]
fn test_multi_leaf_knn_collector() {
    run(test_multi_leaf_knn_collector::class_def);
}

#[test]
fn test_accept_docs() {
    run(test_accept_docs::class_def);
}

#[test]
fn test_bit_set_doc_id_stream() {
    run(test_bit_set_doc_id_stream::class_def);
}

#[test]
fn test_approximation_search_equivalence() {
    run(test_approximation_search_equivalence::class_def);
}

#[test]
fn test_automaton_query() {
    run(test_automaton_query::class_def);
}

#[test]
fn test_automaton_query_unicode() {
    run(test_automaton_query_unicode::class_def);
}

#[test]
fn test_blended_term_query() {
    run(test_blended_term_query::class_def);
}

#[test]
fn test_block_max_conjunction() {
    run(test_block_max_conjunction::class_def);
}

#[test]
fn test_boolean_or() {
    run(test_boolean_or::class_def);
}

#[test]
fn test_boolean2() {
    run(test_boolean2::class_def);
}

#[test]
fn test_boolean_min_should_match() {
    run(test_boolean_min_should_match::class_def);
}

#[test]
fn test_base_range_filter() {
    run(test_base_range_filter::class_def);
}

#[test]
fn test_bayesian_score_query() {
    run(test_bayesian_score_query::class_def);
}

#[test]
fn test_numeric_comparator() {
    run(test_numeric_comparator::class_def);
}

#[test]
fn test_term_ord_val_comparator_adaptive_skipping() {
    run(test_term_ord_val_comparator_adaptive_skipping::class_def);
}

#[test]
fn test_binary_sort_field() {
    run(test_binary_sort_field::class_def);
}

#[test]
fn test_boost_query() {
    run(test_boost_query::class_def);
}

#[test]
fn test_caching_collector() {
    run(test_caching_collector::class_def);
}

#[test]
fn test_date_sort() {
    run(test_date_sort::class_def);
}

#[test]
fn test_conjunctions() {
    run(test_conjunctions::class_def);
}

#[test]
fn test_constant_score_query() {
    run(test_constant_score_query::class_def);
}

#[test]
fn test_collector_manager() {
    run(test_collector_manager::class_def);
}

#[test]
fn test_custom_searcher_sort() {
    run(test_custom_searcher_sort::class_def);
}

#[test]
fn test_constant_score_bulk_scorer() {
    run(test_constant_score_bulk_scorer::class_def);
}

#[test]
fn test_constant_score_scorer() {
    run(test_constant_score_scorer::class_def);
}

#[test]
fn test_conjunction_disi() {
    run(test_conjunction_disi::class_def);
}
