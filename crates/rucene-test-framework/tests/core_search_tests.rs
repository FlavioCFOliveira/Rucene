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

    pub mod base_doc_values_skipper_tests;
    pub mod base_vector_similarity_query_test_case;
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
    pub mod test_boolean_query;
    pub mod test_boolean_query_visit_subscorers;
    pub mod test_boolean_rewrites;
    pub mod test_boolean_scorer;
    pub mod test_boolean_scorer_supplier;
    pub mod test_boost_query;
    pub mod test_byte_vector_similarity_query;
    pub mod test_caching_collector;
    pub mod test_collector_manager;
    pub mod test_combined_field_query;
    pub mod test_conjunction_disi;
    pub mod test_conjunctions;
    pub mod test_constant_score_bulk_scorer;
    pub mod test_constant_score_query;
    pub mod test_constant_score_scorer;
    pub mod test_custom_searcher_sort;
    pub mod test_date_sort;
    pub mod test_dense_conjunction_bulk_scorer;
    pub mod test_disi_priority_queue;
    pub mod test_disjunction_disi_approximation;
    pub mod test_disjunction_max_query;
    pub mod test_disjunction_score_block_boundary_propagator;
    pub mod test_doc_id_set_iterator;
    pub mod test_doc_score_encoder;
    pub mod test_doc_values_ordinal_range_iterator;
    pub mod test_doc_values_ordinal_set_iterator;
    pub mod test_doc_values_queries;
    pub mod test_doc_values_range_iterator;
    pub mod test_doc_values_rewrite_method;
    pub mod test_double_values_source;
    pub mod test_double_values_source_rescorer;
    pub mod test_early_termination;
    pub mod test_elevation_comparator;
    pub mod test_field_cache_rewrite_method;
    pub mod test_field_exists_query;
    pub mod test_filter_weight;
    pub mod test_float_vector_similarity_query;
    pub mod test_full_precision_float_vector_similarity_values_source;
    pub mod test_fuzzy_query;
    pub mod test_fuzzy_term_on_short_terms;
    pub mod test_hnsw_queue_saturation_collector;
    pub mod test_multi_leaf_knn_collector;
    pub mod test_numeric_comparator;
    pub mod test_regexp_random2;
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

#[test]
fn test_byte_vector_similarity_query() {
    run(test_byte_vector_similarity_query::class_def);
}

#[test]
fn test_boolean_query() {
    run(test_boolean_query::class_def);
}

#[test]
fn test_boolean_query_visit_subscorers() {
    run(test_boolean_query_visit_subscorers::class_def);
}

#[test]
fn test_boolean_rewrites() {
    run(test_boolean_rewrites::class_def);
}

#[test]
fn test_boolean_scorer() {
    run(test_boolean_scorer::class_def);
}

#[test]
fn test_boolean_scorer_supplier() {
    run(test_boolean_scorer_supplier::class_def);
}

#[test]
fn test_combined_field_query() {
    run(test_combined_field_query::class_def);
}

#[test]
fn test_disi_priority_queue() {
    run(test_disi_priority_queue::class_def);
}

#[test]
fn test_disjunction_disi_approximation() {
    run(test_disjunction_disi_approximation::class_def);
}

#[test]
fn test_disjunction_score_block_boundary_propagator() {
    run(test_disjunction_score_block_boundary_propagator::class_def);
}

#[test]
fn test_doc_id_set_iterator() {
    run(test_doc_id_set_iterator::class_def);
}

#[test]
fn test_doc_score_encoder() {
    run(test_doc_score_encoder::class_def);
}

#[test]
fn test_filter_weight() {
    run(test_filter_weight::class_def);
}

#[test]
fn test_early_termination() {
    run(test_early_termination::class_def);
}

#[test]
fn test_dense_conjunction_bulk_scorer() {
    run(test_dense_conjunction_bulk_scorer::class_def);
}

#[test]
fn test_disjunction_max_query() {
    run(test_disjunction_max_query::class_def);
}

#[test]
fn test_doc_values_ordinal_range_iterator() {
    run(test_doc_values_ordinal_range_iterator::class_def);
}

#[test]
fn test_doc_values_range_iterator() {
    run(test_doc_values_range_iterator::class_def);
}

#[test]
fn test_doc_values_ordinal_set_iterator() {
    run(test_doc_values_ordinal_set_iterator::class_def);
}

#[test]
fn test_doc_values_queries() {
    run(test_doc_values_queries::class_def);
}

#[test]
fn test_regexp_random2() {
    run(test_regexp_random2::class_def);
}

#[test]
fn test_field_cache_rewrite_method() {
    run(test_field_cache_rewrite_method::class_def);
}

#[test]
fn test_doc_values_rewrite_method() {
    run(test_doc_values_rewrite_method::class_def);
}

#[test]
fn test_double_values_source() {
    run(test_double_values_source::class_def);
}

#[test]
fn test_double_values_source_rescorer() {
    run(test_double_values_source_rescorer::class_def);
}

#[test]
fn test_elevation_comparator() {
    run(test_elevation_comparator::class_def);
}

#[test]
fn test_field_exists_query() {
    run(test_field_exists_query::class_def);
}

#[test]
fn test_float_vector_similarity_query() {
    run(test_float_vector_similarity_query::class_def);
}

#[test]
fn test_full_precision_float_vector_similarity_values_source() {
    run(test_full_precision_float_vector_similarity_values_source::class_def);
}

#[test]
fn test_fuzzy_query() {
    run(test_fuzzy_query::class_def);
}

#[test]
fn test_fuzzy_term_on_short_terms() {
    run(test_fuzzy_term_on_short_terms::class_def);
}

#[test]
fn test_hnsw_queue_saturation_collector() {
    run(test_hnsw_queue_saturation_collector::class_def);
}
