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

//! Port of the test classes of Lucene 10.5.0 `lucene/spatial-extras/src/test`
//! (rmp #632), each run as a suite by the ported runner. The Java packages
//! below `org.apache.lucene.spatial` are the modules of the same names; the
//! test resources (`src/test-files`) are shipped in `tests/test-files`.

// The ports keep Java's statements line by line: indexed loops, float
// literals, late-initialized locals and helper methods with many parameters;
// they keep the Java members no test reads (`SpatialMatchConcern.SUPERSET`,
// unused data-file constants, `RandomizedShapeTestCase`'s static helpers).
#![allow(
    dead_code,
    clippy::needless_range_loop,
    clippy::excessive_precision,
    clippy::too_many_arguments,
    clippy::needless_late_init,
    clippy::new_without_default
)]

#[path = "../../rucene-test-framework/tests/core_support/mod.rs"]
mod core_support;
mod spatial_support;

mod spatial {
    pub mod spatial_match_concern;
    pub mod spatial_test_case;
    pub mod spatial_test_data;
    pub mod spatial_test_query;
    pub mod strategy_test_case;
    pub mod test_distance_strategy;
    pub mod test_ported_solr3;
    pub mod test_query_equals_hash_code;
    pub mod test_spatial_args;
    pub mod test_spatial_example;
    pub mod test_test_framework;
}
mod bbox {
    pub mod test_b_box_strategy;
}
mod composite {
    pub mod test_composite_strategy;
}
mod prefix {
    pub mod cell_to_bytes_ref_iterator50;
    pub mod random_spatial_op_strategy_test_case;
    pub mod test_date_nr_strategy;
    pub mod test_heatmap_facet_counter;
    pub mod test_jts_polygon;
    pub mod test_number_range_facets;
    pub mod test_random_spatial_op_fuzzy_prefix_tree;
    pub mod test_random_spatial_op_fuzzy_prefix_tree50;
    pub mod test_recursive_prefix_tree_strategy;
    pub mod test_term_query_prefix_grid_strategy;
    pub mod tree {
        pub mod test_date_range_prefix_tree;
        pub mod test_s2_prefix_tree;
        pub mod test_spatial_prefix_tree;
    }
}
mod query {
    pub mod test_spatial_args_parser;
}
mod serialized {
    pub mod test_serialized_strategy;
}
mod spatial4j {
    pub mod randomized_shape_test_case;
    pub mod shape_rect_relation_test_case;
    pub mod test_geo3d;
    pub mod test_geo3d_rpt;
    pub mod test_geo3d_shape_sphere_model_rect_relation;
    pub mod test_geo3d_shape_wgs84_model_rect_relation;
}
mod vector {
    pub mod test_point_vector_strategy;
}

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;

/// Runs the suite `class` builds, with the Java exception classes of the
/// spatial modules registered in the runner's class hierarchy.
fn run(class: fn() -> Arc<Class>) {
    spatial_support::register_exception_classes();
    core_support::run(class);
}

macro_rules! suites {
    ($($test:ident => $class:path;)*) => {
        $(
            #[test]
            fn $test() {
                run($class);
            }
        )*
    };
}

suites! {
    test_b_box_strategy => bbox::test_b_box_strategy::class_def;
    test_composite_strategy => composite::test_composite_strategy::class_def;
    test_date_nr_strategy => prefix::test_date_nr_strategy::class_def;
    test_heatmap_facet_counter => prefix::test_heatmap_facet_counter::class_def;
    test_jts_polygon => prefix::test_jts_polygon::class_def;
    test_number_range_facets => prefix::test_number_range_facets::class_def;
    test_random_spatial_op_fuzzy_prefix_tree => prefix::test_random_spatial_op_fuzzy_prefix_tree::class_def;
    test_random_spatial_op_fuzzy_prefix_tree50 => prefix::test_random_spatial_op_fuzzy_prefix_tree50::class_def;
    test_recursive_prefix_tree_strategy => prefix::test_recursive_prefix_tree_strategy::class_def;
    test_term_query_prefix_grid_strategy => prefix::test_term_query_prefix_grid_strategy::class_def;
    test_date_range_prefix_tree => prefix::tree::test_date_range_prefix_tree::class_def;
    test_s2_prefix_tree => prefix::tree::test_s2_prefix_tree::class_def;
    test_spatial_prefix_tree => prefix::tree::test_spatial_prefix_tree::class_def;
    test_spatial_args_parser => query::test_spatial_args_parser::class_def;
    test_serialized_strategy => serialized::test_serialized_strategy::class_def;
    test_distance_strategy => spatial::test_distance_strategy::class_def;
    test_ported_solr3 => spatial::test_ported_solr3::class_def;
    test_query_equals_hash_code => spatial::test_query_equals_hash_code::class_def;
    test_spatial_args => spatial::test_spatial_args::class_def;
    test_spatial_example => spatial::test_spatial_example::class_def;
    test_test_framework => spatial::test_test_framework::class_def;
    test_geo3d => spatial4j::test_geo3d::class_def;
    test_geo3d_rpt => spatial4j::test_geo3d_rpt::class_def;
    test_geo3d_shape_sphere_model_rect_relation => spatial4j::test_geo3d_shape_sphere_model_rect_relation::class_def;
    test_geo3d_shape_wgs84_model_rect_relation => spatial4j::test_geo3d_shape_wgs84_model_rect_relation::class_def;
    test_point_vector_strategy => vector::test_point_vector_strategy::class_def;
}
