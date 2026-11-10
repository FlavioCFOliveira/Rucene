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
    pub mod base_knn_vector_query_test_case;
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
    pub mod test_index_or_doc_values_query;
    pub mod test_index_searcher;
    pub mod test_index_sort_sorted_numeric_doc_values_range_query;
    pub mod test_indri_and_query;
    pub mod test_knn_byte_vector_query;
    pub mod test_knn_float_vector_query;
    pub mod test_late_interaction_float_values_source;
    pub mod test_late_interaction_rescorer;
    pub mod test_live_field_values;
    pub mod test_log_odds_fusion_query;
    pub mod test_long_values_source;
    pub mod test_lru_query_cache;
    pub mod test_match_all_docs_query;
    pub mod test_match_no_docs_query;
    pub mod test_matches_iterator;
    pub mod test_max_clause_limit;
    pub mod test_max_score_accumulator;
    pub mod test_max_score_bulk_scorer;
    pub mod test_min_should_match2;
    pub mod test_multi_collector;
    pub mod test_multi_collector_manager;
    pub mod test_multi_leaf_knn_collector;
    pub mod test_multi_phrase_enum;
    pub mod test_multi_phrase_query;
    pub mod test_multi_slice_merge;
    pub mod test_multi_term_constant_score;
    pub mod test_multi_term_query_rewrites;
    pub mod test_multi_thread_term_vectors;
    pub mod test_multiset;
    pub mod test_nearest;
    pub mod test_needs_scores;
    pub mod test_ngram_phrase_query;
    pub mod test_not;
    pub mod test_numeric_comparator;
    pub mod test_numeric_field_stats;
    pub mod test_patience_byte_vector_query;
    pub mod test_patience_float_vector_query;
    pub mod test_phrase_matcher_contract;
    pub mod test_phrase_prefix_query;
    pub mod test_position_increment;
    pub mod test_positive_scores_only_collector;
    pub mod test_prefix_in_boolean_query;
    pub mod test_prefix_query;
    pub mod test_prefix_random;
    pub mod test_query_rescorer;
    pub mod test_query_visitor;
    pub mod test_range_doc_id_stream;
    pub mod test_range_fields_doc_values_query;
    pub mod test_regexp_query;
    pub mod test_regexp_random;
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

#[test]
fn test_index_or_doc_values_query() {
    run(test_index_or_doc_values_query::class_def);
}

#[test]
fn test_index_searcher() {
    run(test_index_searcher::class_def);
}

#[test]
fn test_index_sort_sorted_numeric_doc_values_range_query() {
    run(test_index_sort_sorted_numeric_doc_values_range_query::class_def);
}

#[test]
fn test_indri_and_query() {
    run(test_indri_and_query::class_def);
}

#[test]
fn test_knn_float_vector_query() {
    run(test_knn_float_vector_query::class_def);
}

#[test]
fn test_knn_byte_vector_query() {
    run(test_knn_byte_vector_query::class_def);
}

#[test]
fn test_knn_byte_vector_query_mmap() {
    run(test_knn_byte_vector_query::mmap_class_def);
}

#[test]
fn test_late_interaction_float_values_source() {
    run(test_late_interaction_float_values_source::class_def);
}

#[test]
fn test_late_interaction_rescorer() {
    run(test_late_interaction_rescorer::class_def);
}

#[test]
fn test_live_field_values() {
    run(test_live_field_values::class_def);
}

#[test]
fn test_log_odds_fusion_query() {
    run(test_log_odds_fusion_query::class_def);
}

#[test]
fn test_long_values_source() {
    run(test_long_values_source::class_def);
}

#[test]
fn test_match_all_docs_query() {
    run(test_match_all_docs_query::class_def);
}

#[test]
fn test_match_no_docs_query() {
    run(test_match_no_docs_query::class_def);
}

#[test]
fn test_max_clause_limit() {
    run(test_max_clause_limit::class_def);
}

#[test]
fn test_max_score_accumulator() {
    run(test_max_score_accumulator::class_def);
}

#[test]
fn test_multiset() {
    run(test_multiset::class_def);
}

#[test]
fn test_multi_slice_merge() {
    run(test_multi_slice_merge::class_def);
}

#[test]
fn test_multi_phrase_enum() {
    run(test_multi_phrase_enum::class_def);
}

#[test]
fn test_multi_term_query_rewrites() {
    run(test_multi_term_query_rewrites::class_def);
}

#[test]
fn test_lru_query_cache() {
    run(test_lru_query_cache::class_def);
}

#[test]
fn test_matches_iterator() {
    run(test_matches_iterator::class_def);
}

#[test]
fn test_max_score_bulk_scorer() {
    run(test_max_score_bulk_scorer::class_def);
}

#[test]
fn test_min_should_match2() {
    run(test_min_should_match2::class_def);
}

#[test]
fn test_multi_collector() {
    run(test_multi_collector::class_def);
}

#[test]
fn test_multi_collector_manager() {
    run(test_multi_collector_manager::class_def);
}

#[test]
fn test_multi_phrase_query() {
    run(test_multi_phrase_query::class_def);
}

#[test]
fn test_multi_term_constant_score() {
    run(test_multi_term_constant_score::class_def);
}

#[test]
fn test_not() {
    run(test_not::class_def);
}

#[test]
fn test_ngram_phrase_query() {
    run(test_ngram_phrase_query::class_def);
}

#[test]
fn test_phrase_prefix_query() {
    run(test_phrase_prefix_query::class_def);
}

#[test]
fn test_prefix_in_boolean_query() {
    run(test_prefix_in_boolean_query::class_def);
}

#[test]
fn test_prefix_query() {
    run(test_prefix_query::class_def);
}

#[test]
fn test_positive_scores_only_collector() {
    run(test_positive_scores_only_collector::class_def);
}

#[test]
fn test_prefix_random() {
    run(test_prefix_random::class_def);
}

#[test]
fn test_regexp_random() {
    run(test_regexp_random::class_def);
}

#[test]
fn test_range_doc_id_stream() {
    run(test_range_doc_id_stream::class_def);
}

#[test]
fn test_multi_thread_term_vectors() {
    run(test_multi_thread_term_vectors::class_def);
}

#[test]
fn test_patience_byte_vector_query() {
    run(test_patience_byte_vector_query::class_def);
}

#[test]
fn test_patience_float_vector_query() {
    run(test_patience_float_vector_query::class_def);
}

#[test]
fn test_regexp_query() {
    run(test_regexp_query::class_def);
}

#[test]
fn test_needs_scores() {
    run(test_needs_scores::class_def);
}

#[test]
fn test_position_increment() {
    run(test_position_increment::class_def);
}

#[test]
fn test_phrase_matcher_contract() {
    run(test_phrase_matcher_contract::class_def);
}

#[test]
fn test_range_fields_doc_values_query() {
    run(test_range_fields_doc_values_query::class_def);
}

#[test]
fn test_numeric_field_stats() {
    run(test_numeric_field_stats::class_def);
}

#[test]
fn test_nearest() {
    run(test_nearest::class_def);
}

#[test]
fn test_query_visitor() {
    run(test_query_visitor::class_def);
}

#[test]
fn test_query_rescorer() {
    run(test_query_rescorer::class_def);
}
