//! Port of the tests of `lucene/core/src/test/org/apache/lucene/document/`
//! (sprint 9, tasks #372 and #373). Each class runs as a suite under
//! `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

#[path = "core_document/base_shape_encoding_test_case.rs"]
mod base_shape_encoding_test_case;
#[path = "core_document/base_spatial_test_case.rs"]
mod base_spatial_test_case;
mod core_support;
#[path = "core_document/spatial_shape_queries.rs"]
mod spatial_shape_queries;
#[path = "core_document/test_lat_lon_shape.rs"]
mod test_lat_lon_shape;
#[path = "core_document/test_shape_doc_values.rs"]
mod test_shape_doc_values;
#[path = "core_document/test_xy_shape.rs"]
mod test_xy_shape;
#[path = "core_document/column"]
mod column {
    pub mod column_batch_test_util;
    pub mod test_column_batch_binary_column;
    pub mod test_column_batch_dictionary_column;
    pub mod test_column_batch_long_column;
    pub mod test_column_batch_multi_column_field;
    pub mod test_column_batch_token_stream_column;
    pub mod test_column_batch_vector_column;
}
#[path = "core_document/test_binary_document.rs"]
mod test_binary_document;
#[path = "core_document/test_date_tools.rs"]
mod test_date_tools;
#[path = "core_document/test_doc_values_long_hash_set.rs"]
mod test_doc_values_long_hash_set;
#[path = "core_document/test_double_range.rs"]
mod test_double_range;
#[path = "core_document/test_field_type.rs"]
mod test_field_type;
#[path = "core_document/test_float_range.rs"]
mod test_float_range;
#[path = "core_document/test_int_range.rs"]
mod test_int_range;
#[path = "core_document/test_keyword_field.rs"]
mod test_keyword_field;
#[path = "core_document/test_lat_lon_doc_values_field.rs"]
mod test_lat_lon_doc_values_field;
#[path = "core_document/test_lat_lon_point.rs"]
mod test_lat_lon_point;
#[path = "core_document/test_long_range.rs"]
mod test_long_range;

#[path = "core_document/test_document.rs"]
mod test_document;
#[path = "core_document/test_feature_double_values.rs"]
mod test_feature_double_values;
#[path = "core_document/test_feature_field.rs"]
mod test_feature_field;
#[path = "core_document/test_feature_sort.rs"]
mod test_feature_sort;
#[path = "core_document/test_field.rs"]
mod test_field;
#[path = "core_document/test_inet_address_point.rs"]
mod test_inet_address_point;
#[path = "core_document/test_lat_lon_point_distance_feature_query.rs"]
mod test_lat_lon_point_distance_feature_query;
#[path = "core_document/test_lat_lon_point_distance_sort.rs"]
mod test_lat_lon_point_distance_sort;
#[path = "core_document/test_lat_lon_shape_encoding.rs"]
mod test_lat_lon_shape_encoding;
#[path = "core_document/test_late_interaction_field.rs"]
mod test_late_interaction_field;
#[path = "core_document/test_long_distance_feature_query.rs"]
mod test_long_distance_feature_query;
#[path = "core_document/test_many_knn_docs.rs"]
mod test_many_knn_docs;
#[path = "core_document/test_per_field_consistency.rs"]
mod test_per_field_consistency;
#[path = "core_document/test_range_filtered_bulk_scorer.rs"]
mod test_range_filtered_bulk_scorer;
#[path = "core_document/test_sorted_set_doc_values_set_query.rs"]
mod test_sorted_set_doc_values_set_query;
#[path = "core_document/test_sorted_skipper_scorer_supplier.rs"]
mod test_sorted_skipper_scorer_supplier;
#[path = "core_document/test_xy_shape_encoding.rs"]
mod test_xy_shape_encoding;
use core_support::run;

#[test]
fn test_column_batch_binary_column() {
    run(column::test_column_batch_binary_column::class_def);
}

#[test]
fn test_column_batch_dictionary_column() {
    run(column::test_column_batch_dictionary_column::class_def);
}

#[test]
fn test_column_batch_long_column() {
    run(column::test_column_batch_long_column::class_def);
}

#[test]
fn test_column_batch_multi_column_field() {
    run(column::test_column_batch_multi_column_field::class_def);
}

#[test]
fn test_column_batch_token_stream_column() {
    run(column::test_column_batch_token_stream_column::class_def);
}

#[test]
fn test_column_batch_vector_column() {
    run(column::test_column_batch_vector_column::class_def);
}

#[test]
fn test_binary_document() {
    run(test_binary_document::class_def);
}

#[test]
fn test_date_tools() {
    run(test_date_tools::class_def);
}

#[test]
fn test_doc_values_long_hash_set() {
    run(test_doc_values_long_hash_set::class_def);
}

#[test]
fn test_double_range() {
    run(test_double_range::class_def);
}

#[test]
fn test_field_type() {
    run(test_field_type::class_def);
}

#[test]
fn test_float_range() {
    run(test_float_range::class_def);
}

#[test]
fn test_int_range() {
    run(test_int_range::class_def);
}

#[test]
fn test_keyword_field() {
    run(test_keyword_field::class_def);
}

#[test]
fn test_lat_lon_doc_values_field() {
    run(test_lat_lon_doc_values_field::class_def);
}

#[test]
fn test_lat_lon_point() {
    run(test_lat_lon_point::class_def);
}

#[test]
fn test_long_range() {
    run(test_long_range::class_def);
}

#[test]
fn test_document() {
    run(test_document::class_def);
}

#[test]
fn test_feature_double_values() {
    run(test_feature_double_values::class_def);
}

#[test]
fn test_feature_field() {
    run(test_feature_field::class_def);
}

#[test]
fn test_feature_sort() {
    run(test_feature_sort::class_def);
}

#[test]
fn test_field() {
    run(test_field::class_def);
}

#[test]
fn test_inet_address_point() {
    run(test_inet_address_point::class_def);
}

#[test]
fn test_late_interaction_field() {
    run(test_late_interaction_field::class_def);
}

#[test]
fn test_sorted_set_doc_values_set_query() {
    run(test_sorted_set_doc_values_set_query::class_def);
}

#[test]
fn test_sorted_skipper_scorer_supplier() {
    run(test_sorted_skipper_scorer_supplier::class_def);
}

#[test]
fn test_range_filtered_bulk_scorer() {
    run(test_range_filtered_bulk_scorer::class_def);
}

#[test]
fn test_many_knn_docs() {
    run(test_many_knn_docs::class_def);
}

#[test]
fn test_per_field_consistency() {
    run(test_per_field_consistency::class_def);
}

#[test]
fn test_long_distance_feature_query() {
    run(test_long_distance_feature_query::class_def);
}

#[test]
fn test_lat_lon_point_distance_sort() {
    run(test_lat_lon_point_distance_sort::class_def);
}

#[test]
fn test_lat_lon_point_distance_feature_query() {
    run(test_lat_lon_point_distance_feature_query::class_def);
}

#[test]
fn test_lat_lon_shape_encoding() {
    run(test_lat_lon_shape_encoding::class_def);
}

#[test]
fn test_xy_shape_encoding() {
    run(test_xy_shape_encoding::class_def);
}

#[test]
fn test_lat_lon_point_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_point_shape_queries::class_def);
}

#[test]
fn test_lat_lon_line_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_line_shape_queries::class_def);
}

#[test]
fn test_lat_lon_polygon_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_polygon_shape_queries::class_def);
}

#[test]
fn test_lat_lon_multi_point_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_multi_point_shape_queries::class_def);
}

#[test]
fn test_lat_lon_multi_line_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_multi_line_shape_queries::class_def);
}

#[test]
fn test_lat_lon_multi_polygon_shape_queries() {
    run(spatial_shape_queries::test_lat_lon_multi_polygon_shape_queries::class_def);
}

#[test]
fn test_lat_lon_point_point_queries() {
    run(spatial_shape_queries::test_lat_lon_point_point_queries::class_def);
}

#[test]
fn test_lat_lon_multi_point_point_queries() {
    run(spatial_shape_queries::test_lat_lon_multi_point_point_queries::class_def);
}

#[test]
fn test_lat_lon_doc_values_point_point_queries() {
    run(spatial_shape_queries::test_lat_lon_doc_values_point_point_queries::class_def);
}

#[test]
fn test_lat_lon_doc_values_multi_point_point_queries() {
    run(spatial_shape_queries::test_lat_lon_doc_values_multi_point_point_queries::class_def);
}

#[test]
fn test_lat_lon_line_shape_dv_queries() {
    run(spatial_shape_queries::test_lat_lon_line_shape_dv_queries::class_def);
}

#[test]
fn test_lat_lon_point_shape_dv_queries() {
    run(spatial_shape_queries::test_lat_lon_point_shape_dv_queries::class_def);
}

#[test]
fn test_lat_lon_polygon_shape_dv_queries() {
    run(spatial_shape_queries::test_lat_lon_polygon_shape_dv_queries::class_def);
}

#[test]
fn test_xy_point_shape_queries() {
    run(spatial_shape_queries::test_xy_point_shape_queries::class_def);
}

#[test]
fn test_xy_line_shape_queries() {
    run(spatial_shape_queries::test_xy_line_shape_queries::class_def);
}

#[test]
fn test_xy_polygon_shape_queries() {
    run(spatial_shape_queries::test_xy_polygon_shape_queries::class_def);
}

#[test]
fn test_xy_multi_point_shape_queries() {
    run(spatial_shape_queries::test_xy_multi_point_shape_queries::class_def);
}

#[test]
fn test_xy_multi_line_shape_queries() {
    run(spatial_shape_queries::test_xy_multi_line_shape_queries::class_def);
}

#[test]
fn test_xy_multi_polygon_shape_queries() {
    run(spatial_shape_queries::test_xy_multi_polygon_shape_queries::class_def);
}

#[test]
fn test_xy_line_shape_dv_queries() {
    run(spatial_shape_queries::test_xy_line_shape_dv_queries::class_def);
}

#[test]
fn test_xy_point_shape_dv_queries() {
    run(spatial_shape_queries::test_xy_point_shape_dv_queries::class_def);
}

#[test]
fn test_xy_polygon_shape_dv_queries() {
    run(spatial_shape_queries::test_xy_polygon_shape_dv_queries::class_def);
}

#[test]
fn test_lat_lon_shape() {
    run(test_lat_lon_shape::class_def);
}

#[test]
fn test_xy_shape() {
    run(test_xy_shape::class_def);
}

#[test]
fn test_shape_doc_values() {
    run(test_shape_doc_values::class_def);
}
