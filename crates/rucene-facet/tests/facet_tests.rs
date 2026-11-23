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

//! Port of the test classes of Lucene 10.5.0 `lucene/facet/src/test`
//! (rmp #630), each run as a suite by the ported runner.

mod facet_support;

mod root {
    pub mod slow_directory;
    pub mod test_drill_down_query;
    pub mod multi_values_source_test_case;
    pub mod test_long_value_facet_counts;
    pub mod test_multi_double_values_source;
    pub mod test_multi_long_values_source;
    pub mod test_multiple_index_fields;
    pub mod test_random_sampling_facets_collector;
    pub mod test_string_value_facet_counts;
    pub mod test_facet_query;
    pub mod test_facet_utils;
    pub mod test_facets_config;
}

mod taxonomy {
    pub mod directory {
        pub mod test_add_taxonomy;
        pub mod test_concurrent_faceted_indexing;
        pub mod test_directory_taxonomy_reader;
        pub mod test_directory_taxonomy_writer;
        pub mod test_taxonomy_index_arrays;
    }
    pub mod test_facet_label;
    pub mod test_ordinal_data;
    pub mod test_ordinal_mapping_leaf_reader;
    pub mod test_searcher_taxonomy_manager;
    pub mod test_taxonomy_combined;
    pub mod test_taxonomy_facet_counts2;
    pub mod test_taxonomy_facet_labels;
    pub mod test_lru_hash_map;
    pub mod writercache {
        pub mod test_2gb_char_block_array;
        pub mod test_char_block_array;
        pub mod test_lru_taxonomy_writer_cache;
    }
}

use facet_support::core_support::run;

macro_rules! suites {
    ($($name:ident => $path:path),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run($path);
            }
        )*
    };
}

suites! {
    test_drill_down_query => root::test_drill_down_query::class_def,
    test_facet_query => root::test_facet_query::class_def,
    test_facet_utils => root::test_facet_utils::class_def,
    test_facets_config => root::test_facets_config::class_def,
    test_long_value_facet_counts => root::test_long_value_facet_counts::class_def,
    test_multi_double_values_source => root::test_multi_double_values_source::class_def,
    test_multi_long_values_source => root::test_multi_long_values_source::class_def,
    test_multiple_index_fields => root::test_multiple_index_fields::class_def,
    test_random_sampling_facets_collector => root::test_random_sampling_facets_collector::class_def,
    test_string_value_facet_counts => root::test_string_value_facet_counts::class_def,
    test_facet_label => taxonomy::test_facet_label::class_def,
    test_taxonomy_facet_labels => taxonomy::test_taxonomy_facet_labels::class_def,
    test_ordinal_data => taxonomy::test_ordinal_data::class_def,
    test_ordinal_mapping_leaf_reader => taxonomy::test_ordinal_mapping_leaf_reader::class_def,
    test_concurrent_faceted_indexing => taxonomy::directory::test_concurrent_faceted_indexing::class_def,
    test_add_taxonomy => taxonomy::directory::test_add_taxonomy::class_def,
    test_taxonomy_index_arrays => taxonomy::directory::test_taxonomy_index_arrays::class_def,
    test_taxonomy_facet_counts2 => taxonomy::test_taxonomy_facet_counts2::class_def,
    test_taxonomy_combined => taxonomy::test_taxonomy_combined::class_def,
    test_searcher_taxonomy_manager => taxonomy::test_searcher_taxonomy_manager::class_def,
    test_directory_taxonomy_writer => taxonomy::directory::test_directory_taxonomy_writer::class_def,
    test_directory_taxonomy_reader => taxonomy::directory::test_directory_taxonomy_reader::class_def,
    test_lru_hash_map => taxonomy::test_lru_hash_map::class_def,
    test_2gb_char_block_array => taxonomy::writercache::test_2gb_char_block_array::class_def,
    test_char_block_array => taxonomy::writercache::test_char_block_array::class_def,
    test_lru_taxonomy_writer_cache => taxonomy::writercache::test_lru_taxonomy_writer_cache::class_def,
}
